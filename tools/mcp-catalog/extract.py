#!/usr/bin/env python3
"""Extract the MCP tool catalog from beats-btw-dj's Control/MCPBridge.swift, byte for byte.

The Swift source is the only place the tool text lives (descriptions, schemas, server
instructions, the tool -> control-method map). This evaluates the small Swift subset that file
uses (string literals incl. multi-line `\"\"\"` with `\\` continuations, `+`, `prop(...)`,
`obj([...])`, `.string(...)`, dictionary/array literals, `tool(...)`) and writes
crates/djengine/data/mcp-tools.json, which the Rust port embeds.

    tools/mcp-catalog/extract.py [path/to/beats-btw-dj] > crates/djengine/data/mcp-tools.json

Read-only with respect to the Swift checkout. Run it again after the Swift catalog changes;
`tests/mcp.rs` pins the result's sha256 so a stale copy is a visible diff, not a silent one.
"""
import json
import re
import subprocess
import sys
from collections import OrderedDict

root = sys.argv[1] if len(sys.argv) > 1 else "../beats-btw-dj"
src = open(f"{root}/Sources/DJCore/Control/MCPBridge.swift", encoding="utf-8").read()
fx = open(f"{root}/Sources/DJCore/Audio/FxUnit.swift", encoding="utf-8").read()

# FxParam ids, in declaration order (the one interpolation in dj_perform's description).
m = re.search(r"enum FxParam: String, CaseIterable, Sendable \{(.*?)\n  public var id", fx, re.S)
fx_ids = []
for line in m.group(1).splitlines():
    c = re.match(r'\s*case (\w+)(?: = "([^"]+)")?\s*$', line)
    if c:
        fx_ids.append(c.group(2) or c.group(1))
FX_IDS = "|".join(fx_ids)


class Lexer:
    def __init__(self, s):
        self.s, self.i, self.toks = s, 0, []
        self.run()

    def run(self):
        s = self.s
        while self.i < len(s):
            c = s[self.i]
            if c.isspace():
                self.i += 1
            elif s.startswith("//", self.i):
                while self.i < len(s) and s[self.i] != "\n":
                    self.i += 1
            elif s.startswith('"""', self.i):
                self.toks.append(("str", self.multiline()))
            elif c == '"':
                self.toks.append(("str", self.single()))
            elif c.isalpha() or c == "_":
                j = self.i
                while j < len(s) and (s[j].isalnum() or s[j] == "_"):
                    j += 1
                self.toks.append(("id", s[self.i:j]))
                self.i = j
            elif c.isdigit():
                j = self.i
                while j < len(s) and (s[j].isdigit() or s[j] == "."):
                    j += 1
                self.toks.append(("num", s[self.i:j]))
                self.i = j
            else:
                self.toks.append(("p", c))
                self.i += 1

    def escape(self, s, i):
        """s[i] is the char after a backslash: return (text, new_i)."""
        c = s[i]
        simple = {"n": "\n", "t": "\t", "\\": "\\", '"': '"', "'": "'", "0": "\0", "r": "\r"}
        if c in simple:
            return simple[c], i + 1
        if c == "(":
            depth, j = 1, i + 1
            while depth:
                if s[j] == '"':  # a nested string literal
                    j += 1
                    while s[j] != '"':
                        j += 2 if s[j] == "\\" else 1
                elif s[j] == "(":
                    depth += 1
                elif s[j] == ")":
                    depth -= 1
                j += 1
            expr = s[i + 1:j - 1]
            if expr.startswith("FxParam.allCases"):
                return FX_IDS, j
            raise SystemExit(f"unhandled interpolation \\({expr})")
        raise SystemExit(f"unhandled escape \\{c}")

    def single(self):
        s, i, out = self.s, self.i + 1, []
        while s[i] != '"':
            if s[i] == "\\":
                t, i = self.escape(s, i + 1)
                out.append(t)
            else:
                out.append(s[i])
                i += 1
        self.i = i + 1
        return "".join(out)

    def multiline(self):
        s = self.s
        i = self.i + 3
        assert s[i] == "\n", "opening \"\"\" must end its line"
        i += 1
        # find the closing delimiter (an unescaped """)
        j = i
        while not s.startswith('"""', j):
            j += 2 if s[j] == "\\" else 1
        body = s[i:j]
        self.i = j + 3
        lines = body.split("\n")
        indent = lines[-1]  # whitespace before the closing """
        assert indent.strip() == "", "closing \"\"\" must be on its own line"
        lines = lines[:-1]
        out, cont = [], False
        for ln in lines:
            if ln.startswith(indent):
                ln = ln[len(indent):]
            elif ln.strip():
                raise SystemExit("under-indented multi-line string line")
            else:
                ln = ""
            # process escapes on this line; a trailing backslash joins the next line
            k, buf, joined = 0, [], False
            while k < len(ln):
                if ln[k] == "\\":
                    if k == len(ln) - 1:
                        joined = True
                        k += 1
                        continue
                    t, k = self.escape(ln, k + 1)
                    buf.append(t)
                else:
                    buf.append(ln[k])
                    k += 1
            out.append(("".join(buf), joined))
        text = ""
        for n, (ln, joined) in enumerate(out):
            text += ln
            if n < len(out) - 1 and not joined:
                text += "\n"
        return text


class Parser:
    def __init__(self, toks):
        self.t, self.i = toks, 0

    def peek(self, k=0):
        return self.t[self.i + k] if self.i + k < len(self.t) else (None, None)

    def eat(self, kind=None, val=None):
        tok = self.t[self.i]
        if (kind and tok[0] != kind) or (val and tok[1] != val):
            raise SystemExit(f"parse error at token {self.i}: wanted {kind} {val}, got {tok}")
        self.i += 1
        return tok

    def expr(self):
        v = self.term()
        while self.peek() == ("p", "+"):
            self.eat()
            w = self.term()
            assert isinstance(v, str) and isinstance(w, str), "+ on non-strings"
            v += w
        return v

    def args(self):
        """( a, b, label: c ) -> (positional list, labelled dict)."""
        self.eat("p", "(")
        pos, lab = [], {}
        while self.peek() != ("p", ")"):
            if self.peek()[0] == "id" and self.peek(1) == ("p", ":"):
                name = self.eat()[1]
                self.eat()
                lab[name] = self.expr()
            else:
                pos.append(self.expr())
            if self.peek() == ("p", ","):
                self.eat()
        self.eat("p", ")")
        return pos, lab

    def term(self):
        kind, val = self.peek()
        if kind == "str":
            self.eat()
            return val
        if kind == "p" and val == "(":  # tuple
            self.eat()
            items = [self.expr()]
            while self.peek() == ("p", ","):
                self.eat()
                if self.peek() == ("p", ")"):
                    break
                items.append(self.expr())
            self.eat("p", ")")
            return ("tuple", items)
        if kind == "p" and val == "[":
            return self.collection()
        if kind == "p" and val == ".":  # .string(x) / .array(x)
            self.eat()
            name = self.eat("id")[1]
            pos, _ = self.args()
            if name == "string":
                return pos[0]
            if name == "array":
                return pos[0]
            raise SystemExit(f"unhandled .{name}")
        if kind == "id":
            self.eat()
            pos, lab = self.args() if self.peek() == ("p", "(") else ([], {})
            if val == "prop":
                return OrderedDict([("type", pos[0]), ("description", pos[1])])
            if val == "obj":
                return OrderedDict((k, v) for k, v in pos[0])
            if val == "tool":
                name, desc, props = pos[0], pos[1], pos[2]
                schema = OrderedDict([("type", "object")])
                schema["properties"] = OrderedDict(sorted(props.items(), key=lambda kv: kv[0]))
                if lab.get("required"):
                    schema["required"] = lab["required"]
                return OrderedDict([("name", name), ("description", desc), ("inputSchema", schema)])
            raise SystemExit(f"unhandled call {val}")
        raise SystemExit(f"unexpected token {self.peek()} at {self.i}")

    def collection(self):
        self.eat("p", "[")
        if self.peek() == ("p", ":"):
            self.eat()
            self.eat("p", "]")
            return OrderedDict()
        items, is_dict, d = [], False, OrderedDict()
        while self.peek() != ("p", "]"):
            a = self.expr()
            if self.peek() == ("p", ":"):
                self.eat()
                is_dict = True
                d[a] = self.expr()
            else:
                items.append(a)
            if self.peek() == ("p", ","):
                self.eat()
        self.eat("p", "]")
        return d if is_dict else items


def find_after(marker):
    k = src.index(marker)
    return src[k:]


# --- the tool -> method map ---
body = find_after("static let methods: [String: String] = [")
body = body[body.index("[") : body.index("\n  ]\n") + 4]
methods = OrderedDict(re.findall(r'"(dj_\w+)": "(\w+)"', body))

# --- the instructions ---
ins_src = find_after("static let instructions = ")
ins_src = ins_src[len("static let instructions = ") :]
ins_src = ins_src[: ins_src.index("\n    \"\"\"\n") + len("\n    \"\"\"\n")]
instructions = Parser(Lexer(ins_src).toks).expr()

# --- the tools ---
tools_src = find_after("static let tools: [JSONValue] = ")
tools_src = tools_src[len("static let tools: [JSONValue] = ") :]
end = tools_src.index("\n  ]\n\n  static func tool(")
tools_src = tools_src[: end + 4]
tools = Parser(Lexer(tools_src).toks).collection()

assert sorted(t["name"] for t in tools) == sorted(methods), "tools != method map keys"
rev = subprocess.run(
    ["git", "-C", root, "rev-parse", "HEAD"], capture_output=True, text=True
).stdout.strip()
out = OrderedDict(
    [
        ("source", "Sources/DJCore/Control/MCPBridge.swift"),
        ("commit", rev),
        ("instructions", instructions),
        ("methods", methods),
        ("tools", tools),
    ]
)
json.dump(out, sys.stdout, ensure_ascii=False, indent=1)
sys.stdout.write("\n")
