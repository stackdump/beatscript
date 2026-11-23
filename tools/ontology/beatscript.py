#!/usr/bin/env python3
"""Generate crates/beatscript-serve/ontology/generated/beatscript.ttl: the BeatScript vocabulary, from what beatscript-serve itself has.

    python3 -I tools/ontology/beatscript.py SOURCES.json crates/beatscript-serve/ontology > crates/beatscript-serve/ontology/generated/beatscript.ttl

(crates/beatscript-serve/ontology/build.sh runs it.) A port of ontology-bitwrap-io's extract/beatscript.py into the BeatScript
ontology, which forked from the bitwrap ontology 0.2.0: the terms are minted under https://beatscript.app/ontology/.
The second argument is the ontology directory: its fork.ttl and the vendored upstream bitwrap release
(upstream/bitwrap-0.2.0/ns.nt) say which terms are already written by hand.

The input is the dump beatscript-serve (the beatscript.app server) writes from its own code (its ontology vocab module:
`ONTOLOGY_VOCAB_OUT=... cargo test -p beatscript-serve --bin beatscript-serve vocab_dump`), so nothing here is typed by hand twice:

  grammar_ebnf     the language's one grammar (grammar.ebnf in the BeatScript engine): a term per production, keyword and token class
  versions         the language's versions (djlive::version): a term each, with its version line, what its program CIDs are
                   the CIDs of and the version it revises, and the language's current version
  instruments      what /api/instruments and the instruments tool answer: voices, kits,
                   families, palettes
  control_nouns    the snapshot of the DJ app's control registry: the nouns `do` reaches and the verbs each accepts
  language         the language engine's registries: generators, pattern transforms, evolve and mutation operators, sugar words, phrase templates
  mcp              both MCP servers' tools/list: BeatScript's (beatscript-serve /mcp) and the ontology's (/ontology/mcp)
  phonemes         the CC0 phoneme set (optional until it lands)
  phrasebook       the public-domain phrasebook's closed vocabularies: its meter families and feels (the corpus itself is data, not vocabulary)
  forms, themes    the form templates (ns:Form, linked to their mutation operators) and the theme library's roles and modes

The output is deterministic: the same dump gives the same bytes. Terms already written by hand (in fork.ttl, or in
the bitwrap release the fork derives from: a transform, an instrument, the describe verb) keep their hand-written
label and definition; this adds only facts.
A drift test in beatscript-serve fails when the release and beatscript-serve's code disagree.
"""
import json
import re
import sys
from pathlib import Path

NS = "https://beatscript.app/ontology/ns#"
BASE = "https://beatscript.app/ontology/"
UPSTREAM_BASE = "https://ontology.bitwrap.io/"
RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
SKOS = "http://www.w3.org/2004/02/skos/core#"

# A keyword's term name: a word is itself; a symbol gets a name. A terminal with no name here stops the build, so a
# new symbol in the grammar is named on purpose, not by accident.
SYMBOLS = {
    "~": "tilde", "@": "at", "|": "bar", "(": "open-paren", ")": "close-paren", ",": "comma", "[": "open-bracket",
    "]": "close-bracket", "<": "lt", ">": "gt", "{": "open-brace", "}": "close-brace", "*": "star", "!": "bang",
    "^": "caret", "?": "question", "/": "slash", "%": "percent", "+": "plus", "-": "minus", "->": "arrow",
    "..": "range", ":": "colon", "=": "equals", "==": "eq", "!=": "ne", "<=": "le", ">=": "ge", "'": "apostrophe",
    "±": "plus-minus", ".": "dot", "#": "hash", "&": "ampersand",
}

# What a few terminals mean where they occur: (production, terminal) -> (concept slug, label, definition). These are
# the pattern modifiers and the mutation sigils; each is emitted only while the grammar still has that terminal in that
# production.
MEANINGS = [
    ("term", "*", "modifier-repeat", "repeat (*n)", "A term written x*n plays n times in the time of one slot: hihat x*4 is four hits in one step."),
    ("term", "?", "modifier-chance", "chance (?p)", "A term written x?p plays with probability p (0 to 1, or a percent), drawn from the program's seed so the same program always plays the same."),
    ("mark", "!", "modifier-accent", "accent (!)", "A term written c2! is accented: played louder than its neighbours."),
    ("mark", "^", "modifier-slide", "slide (^)", "A term written e2^ slides into the next note instead of re-attacking, as on an acid bass line."),
    ("part", "/", "modifier-cycle", "cycle length (/n)", "A part written hihat/12 has a cycle of n sixteenths instead of 16, so it drifts against the bar."),
    ("mutation", "~", "mutation-from-here", "mutation from here on (~)", "An arrange mutation written ~target op applies from that point of the arrangement to its end."),
    ("mutation", "@", "mutation-next-section", "mutation for one section (@)", "An arrange mutation written @target op applies to the next section only."),
]

GENERATOR_SEE_ALSO = {"e": "EuclidTerm", "euclid": "EuclidTerm", "chord": "ChordTerm", "arp": "ArpTerm"}


def lit(s, lang="en"):
    s = s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")
    return f'"{s}"@{lang}' if lang else f'"{s}"'


def typed(v, dt):
    return f'"{v}"^^<http://www.w3.org/2001/XMLSchema#{dt}>'


def iri(path):
    return f"<{BASE}{path}>"


def nsx(local):
    return f"<{NS}{local}>"


def slug(s):
    out = re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,80}", out):
        sys.exit(f"cannot make a term name of {s!r}")
    return out


# ---------- the grammar -------------------------------------------------------

def scan(text):
    """The grammar file as (kind, text, offset): "code" runs and "comment" bodies, quote-aware."""
    out, i, buf, start = [], 0, [], 0
    while i < len(text):
        c = text[i]
        if c == '"':
            j = text.index('"', i + 1)
            buf.append(text[i:j + 1])
            i = j + 1
        elif text.startswith("(*", i):
            if buf:
                out.append(("code", "".join(buf), start))
                buf = []
            j = text.index("*)", i + 2)
            out.append(("comment", text[i + 2:j], i))
            i = j + 2
            start = i
        else:
            if not buf:
                start = i
            buf.append(c)
            i += 1
    if buf:
        out.append(("code", "".join(buf), start))
    return out


def norm(s):
    return " ".join(s.split())


def productions(text):
    """[(name, rhs, section, notes)] in file order. rhs: the right side with comments removed and whitespace collapsed
    (the rule beatscript-serve's drift test applies too). notes: the comments inside the production or after its `;` on the
    same line; a `(* == Title == *)` comment before it names its section."""
    parts = scan(text)
    # comments blanked at their own length (newlines kept), so offsets in `code` are offsets in `text`
    code = "".join(t if k == "code" else re.sub(r"[^\n]", " ", "(*" + t + "*)") for k, t, _ in parts)
    comments = [(o, t) for k, t, o in parts if k == "comment"]
    prods, section, i, start, prev_end = [], "", 0, 0, -1
    while i < len(code):
        c = code[i]
        if c == '"':
            i = code.index('"', i + 1) + 1
            continue
        if c != ";":
            i += 1
            continue
        body = code[start:i]
        name, eq, rhs = body.partition("=")
        name = name.strip()
        if not eq or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
            sys.exit(f"grammar: cannot read a production from {body.strip()[:60]!r}")
        own = start + len(body) - len(body.lstrip())
        line_end = text.find("\n", i)
        line_end = len(text) if line_end < 0 else line_end
        notes = []
        for o, t in comments:
            if prev_end < o < own:
                m = re.fullmatch(r"\s*==\s*(.*?)\s*==\s*", t)
                if m:
                    section = m.group(1)
            elif own <= o < line_end:
                notes.append(norm(t))
        prods.append((name, norm(rhs), section, notes))
        prev_end = line_end
        start = i + 1
        i += 1
    return prods


def rhs_symbols(rhs):
    terminals = re.findall(r'"([^"]*)"', rhs)
    names = re.findall(r"[A-Za-z_][A-Za-z0-9_]*", re.sub(r'"[^"]*"', " ", rhs))
    return terminals, names


def keyword_slug(t):
    if t in SYMBOLS:
        return "symbol-" + SYMBOLS[t]
    if t.endswith("=") and re.fullmatch(r"[a-z]+=", t):
        return slug(t[:-1]) + "-param"
    if re.fullmatch(r"[A-Za-z0-9_-]+", t):
        return slug(t)
    sys.exit(f"grammar: the terminal {t!r} has no term name: add it to SYMBOLS in tools/ontology/beatscript.py")


def production_slug(name):
    # an all-capitals production is lexical (TIME, MOVE, SPEC, TRANSITION); it may share its name with a lower-case
    # one (TIME and time), so it gets its own prefix
    return ("grammar-lexical-" if name.isupper() else "grammar-") + slug(name)


# ---------- output ------------------------------------------------------------

class Out:
    def __init__(self, defined):
        self.defined = defined
        self.blocks = []
        self.count = {}

    def facts(self, subject, facts):
        """Facts about a subject written by hand elsewhere (fork.ttl): no types, label or definition."""
        self.blocks.append(f"{subject} " + " ;\n    ".join(f"{p} {o}" for p, o in facts) + " .")

    def term(self, subject, types, label, definition, facts, kind):
        """One subject: types, label and definition (left out when they are already written by hand), then facts."""
        lines = [f"{subject} a {', '.join(types)}"]
        hand = subject in self.defined
        if not hand:
            lines.append(f"    <{SKOS}prefLabel> {lit(label)}")
            lines.append(f"    <{SKOS}definition> {lit(definition)}")
        if not hand or subject.startswith(f"<{BASE}instrument/"):
            # the genre generator's instruments carry no status; every hand-written concept: term has its own
            lines.append(f'    {nsx("status")} "stable"')
        lines += [f"    {p} {o}" for p, o in facts]
        self.blocks.append(" ;\n".join(lines) + " .")
        self.count[kind] = self.count.get(kind, 0) + 1

    def text(self, header):
        return header + "\n\n".join(self.blocks) + "\n"


CLASSES = """
# --- classes and properties of the generated vocabulary ----------------------

ns:LanguageTerm a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "BeatScript language term"@en ; ns:status "stable" ;
    rdfs:comment "A piece of the BeatScript language as its grammar states it: a production, a keyword or a token class."@en .
ns:GrammarProduction a owl:Class ; rdfs:subClassOf ns:LanguageTerm ; rdfs:label "grammar production"@en ; ns:status "stable" ;
    rdfs:comment "One production of BeatScript's grammar (grammar.ebnf, ISO 14977 EBNF; beatscript.app/docs/language). Its ns:ebnf is the right-hand side as written."@en .
ns:Keyword a owl:Class ; rdfs:subClassOf ns:LanguageTerm ; rdfs:label "keyword"@en ; ns:status "stable" ;
    rdfs:comment "A terminal of BeatScript's grammar: a word or a symbol written as it is."@en .
ns:TokenClass a owl:Class ; rdfs:subClassOf ns:LanguageTerm ; rdfs:label "token class"@en ; ns:status "stable" ;
    rdfs:comment "A kind of token the lexer recognises, such as NAME, INT or NOTE: named in the grammar, defined by the lexer."@en .
ns:PatternModifier a owl:Class ; rdfs:subClassOf ns:PatternTerm ; rdfs:label "pattern modifier"@en ; ns:status "stable" ;
    rdfs:comment "A mark written after a term or part that changes how it plays: repeat, chance, accent, slide, cycle length."@en .
ns:PatternGenerator a owl:Class ; rdfs:subClassOf ns:PatternTerm ; rdfs:label "pattern generator"@en ; ns:status "stable" ;
    rdfs:comment "A call that makes a pattern from its arguments, such as e(5,8), markov(0.6), chord(am) or choose."@en .
ns:EvolveOperator a owl:Class ; rdfs:subClassOf ns:PatternTransform ; rdfs:label "evolve operator"@en ; ns:status "stable" ;
    rdfs:comment "An operation an evolve block applies to a part every N bars, generation after generation."@en .
ns:MutationOperator a owl:Class ; rdfs:subClassOf ns:PatternTransform ; rdfs:label "arrange mutation operator"@en ; ns:status "stable" ;
    rdfs:comment "An operator of an arrange mutation: it changes a let pattern from one point of the arrangement (~) or for one section (@)."@en .
ns:PhraseTemplate a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "phrase template"@en ; ns:status "stable" ;
    rdfs:comment "A template that compiles a phrase to a whole track: arrange from template(NAME, PHRASE). The phrase is the vocal line (its syllables the line's length, its stress the accents and the beats, its meter the subdivision); the arrangement is the template's own."@en .
ns:BeatScriptVoice a owl:Class ; rdfs:subClassOf ns:Instrument ; rdfs:label "BeatScript voice"@en ; ns:status "stable" ;
    rdfs:comment "A voice a BeatScript part may name with inst=NAME: what /api/instruments lists."@en .
ns:DrumKit a owl:Class ; rdfs:subClassOf ns:Instrument ; rdfs:label "drum kit"@en ; ns:status "stable" ;
    rdfs:comment "A set of drum sounds a part may name with kit=NAME."@en .
ns:VoiceFamily a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "voice family"@en ; ns:status "stable" ;
    rdfs:comment "A family word: an inst= name that contains it plays the family's generic voice."@en .
ns:Palette a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "palette"@en ; ns:status "stable" ;
    rdfs:comment "A named set of sounds a program can choose as a whole."@en .
ns:McpTool a owl:Class ; rdfs:label "MCP tool"@en ; ns:status "stable" ;
    rdfs:comment "One tool of an MCP server: its name, description and input schema, as the server's tools/list gives them."@en ;
    owl:disjointWith ns:MusicConcept, ns:Product .

ns:ebnf a owl:DatatypeProperty ; rdfs:domain ns:GrammarProduction ; rdfs:range xsd:string ; rdfs:label "EBNF"@en ; ns:status "stable" ;
    rdfs:comment "The production's right-hand side in ISO 14977 EBNF, comments removed and whitespace collapsed."@en .
ns:grammarSection a owl:DatatypeProperty ; rdfs:domain ns:GrammarProduction ; rdfs:range xsd:string ; rdfs:label "grammar section"@en ; ns:status "stable" .
ns:usesProduction a owl:ObjectProperty ; rdfs:domain ns:GrammarProduction ; rdfs:range ns:GrammarProduction ; rdfs:label "uses production"@en ; ns:status "stable" .
ns:usesKeyword a owl:ObjectProperty ; rdfs:domain ns:GrammarProduction ; rdfs:range ns:Keyword ; rdfs:label "uses keyword"@en ; ns:status "stable" .
ns:usesToken a owl:ObjectProperty ; rdfs:domain ns:GrammarProduction ; rdfs:range ns:TokenClass ; rdfs:label "uses token"@en ; ns:status "stable" .
ns:keywordText a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:domain ns:Keyword ; rdfs:range xsd:string ; rdfs:label "keyword text"@en ; ns:status "stable" .
ns:hostOnly a owl:DatatypeProperty ; rdfs:domain ns:GrammarProduction ; rdfs:range xsd:boolean ; rdfs:label "host only"@en ; ns:status "stable" ;
    rdfs:comment "True for a production that needs a host's store to be read (saved snippets, saved programs)."@en .
ns:writtenAs a owl:DatatypeProperty ; rdfs:range xsd:string ; rdfs:label "written as"@en ; ns:status "stable" ;
    rdfs:comment "How the term is spelled in a BeatScript program."@en .
ns:markedBy a owl:ObjectProperty ; rdfs:range ns:Keyword ; rdfs:label "marked by"@en ; ns:status "stable" .
ns:inVoiceFamily a owl:ObjectProperty ; rdfs:domain ns:BeatScriptVoice ; rdfs:range ns:VoiceFamily ; rdfs:label "in voice family"@en ; ns:status "stable" .
ns:acceptsVerb a owl:ObjectProperty ; owl:inverseOf ns:actsOn ; rdfs:label "accepts verb"@en ; ns:status "stable" .
ns:defaultVerb a owl:ObjectProperty, owl:FunctionalProperty ; rdfs:domain ns:ControlNoun ; rdfs:range ns:ControlVerb ; rdfs:label "default verb"@en ; ns:status "stable" .
ns:controlParameter a owl:DatatypeProperty ; rdfs:domain ns:ControlNoun ; rdfs:range xsd:string ; rdfs:label "control parameter"@en ; ns:status "stable" .
ns:sugarWord a owl:DatatypeProperty ; rdfs:domain ns:ControlNoun ; rdfs:range xsd:string ; rdfs:label "sugar word"@en ; ns:status "stable" ;
    rdfs:comment "A word a program may write without do to reach this noun: vocoder on is do vocoder on."@en .
ns:providedBy a owl:ObjectProperty, owl:FunctionalProperty ; rdfs:domain ns:McpTool ; rdfs:range ns:McpServer ; rdfs:label "provided by"@en ; ns:status "stable" .
ns:toolName a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:domain ns:McpTool ; rdfs:range xsd:string ; rdfs:label "tool name"@en ; ns:status "stable" .
ns:toolTitle a owl:DatatypeProperty ; rdfs:domain ns:McpTool ; rdfs:range xsd:string ; rdfs:label "tool title"@en ; ns:status "stable" .
ns:inputSchema a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:domain ns:McpTool ; rdfs:range xsd:string ; rdfs:label "input schema"@en ; ns:status "stable" ;
    rdfs:comment "The tool's JSON Schema, as compact JSON with sorted keys."@en .
ns:toolParameter a owl:DatatypeProperty ; rdfs:domain ns:McpTool ; rdfs:range xsd:string ; rdfs:label "tool parameter"@en ; ns:status "stable" .
ns:requiredParameter a owl:DatatypeProperty ; rdfs:subPropertyOf ns:toolParameter ; rdfs:label "required parameter"@en ; ns:status "stable" .
ns:readOnly a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:domain ns:McpTool ; rdfs:range xsd:boolean ; rdfs:label "read-only"@en ; ns:status "stable" ;
    rdfs:comment "True when the tool changes nothing (its readOnlyHint)."@en .
ns:needsToken a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:domain ns:McpTool ; rdfs:range xsd:boolean ; rdfs:label "needs token"@en ; ns:status "stable" ;
    rdfs:comment "True when the tool needs a signed-in caller (an MCP token from beatscript.app)."@en .

[] a owl:AllDisjointClasses ;
    owl:members ( ns:LanguageTerm ns:VoiceFamily ns:Palette ns:Genre ns:Scale ns:Instrument ns:PatternTerm ) .
"""

VERSIONS = """
# --- the language's versions ------------------------------------------------------------------------------------

ns:LanguageVersion a owl:Class ; rdfs:subClassOf ns:ProgrammingLanguage ; rdfs:label "language version"@en ; ns:status "stable" ;
    rdfs:comment "One version of a language: the version line that says a text is in it, how its programs are canonicalized and what their CIDs are the CIDs of. A later version is prov:wasRevisionOf the one before it, and a program written in the earlier one keeps its meaning and its CID."@en .
ns:versionOf a owl:ObjectProperty, owl:FunctionalProperty ; rdfs:domain ns:LanguageVersion ; rdfs:range ns:ProgrammingLanguage ; rdfs:label "version of"@en ; ns:status "stable" .
ns:currentVersion a owl:ObjectProperty, owl:FunctionalProperty ; rdfs:domain ns:ProgrammingLanguage ; rdfs:range ns:LanguageVersion ; rdfs:label "current version"@en ; ns:status "stable" ;
    rdfs:comment "The version new text is written in: what fmt upgrades a program to and what the generators write."@en .
ns:versionLine a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:domain ns:LanguageVersion ; rdfs:range xsd:string ; rdfs:label "version line"@en ; ns:status "stable" ;
    rdfs:comment "The header line (a leading #! line) that says a text is in this version."@en .
ns:programCidFormat a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:domain ns:LanguageVersion ; rdfs:range xsd:integer ; rdfs:label "program CID format"@en ; ns:status "stable" ;
    rdfs:comment "The format of the document a program CID of this version is the CID of: {\\"format\\": this number, \\"text\\": the canonical text, \\"type\\": the version's ns:programCidType}."@en .
ns:programCidType a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:domain ns:LanguageVersion ; rdfs:range xsd:string ; rdfs:label "program CID type"@en ; ns:status "stable" ;
    rdfs:comment "The type of the document a program CID of this version is the CID of."@en .
ns:inLanguageVersion a owl:ObjectProperty, owl:FunctionalProperty ; rdfs:domain ns:Program ; rdfs:range ns:LanguageVersion ; rdfs:label "in language version"@en ; ns:status "stable" ;
    rdfs:comment "The version the program is written in, which the first line of its canonical text tells. A program and its upgrade to a later version play the same but are two texts with two CIDs, so two programs: the upgrade is prov:wasRevisionOf the original, and each has one ns:programCid."@en .
"""

PHRASEBOOK = """
# --- the public-domain phrasebook ---------------------------------------------------------------------------

ns:Phrasebook a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "phrasebook"@en ; ns:status "stable" ;
    rdfs:comment "A curated set of phrases a program sings by id: sing @id on melody. The public-domain one is lines of verse, scripture, speech and work song first published before 1929, as a CC0 file with one CID."@en .
ns:PhrasebookEntry a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "phrasebook entry"@en ; ns:status "stable" ;
    rdfs:comment "One line of a phrasebook: its words, the lyric BeatScript sings them as, where and when it was first published, and its meter as a step pattern."@en .
ns:MeterFamily a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "meter family"@en ; ns:status "stable" ;
    rdfs:comment "A kind of line, by how its stresses fall: the closed set a phrasebook entry is filed under."@en .
ns:Feel a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "feel"@en ; ns:status "stable" ;
    rdfs:comment "How a line's syllables sit on the grid: on the sixteenths, or in groups of three."@en .
ns:Technique a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "composition technique"@en ; ns:status "stable" ;
    rdfs:comment "A way of writing music that a person or an agent can follow, taught by one document and used through the language's own words."@en .
ns:usesPhrasebook a owl:ObjectProperty ; rdfs:domain ns:Technique ; rdfs:range ns:Phrasebook ; rdfs:label "uses phrasebook"@en ; ns:status "stable" ;
    rdfs:comment "The phrasebook a technique takes its found material from."@en .
ns:suitsGroove a owl:DatatypeProperty ; rdfs:domain ns:MeterFamily ; rdfs:range xsd:string ; rdfs:label "suits groove"@en ; ns:status "stable" ;
    rdfs:comment "The groove a line of this meter family sits on, in a sentence: a trochee chants on the kick, an iamb lilts on the offbeat, an anapest wants a triplet feel."@en .
ns:suitsGenre a owl:ObjectProperty ; rdfs:domain ns:MeterFamily ; rdfs:range ns:Genre ; rdfs:label "suits genre"@en ; ns:status "stable" ;
    rdfs:comment "A genre whose groove a line of this meter family sits on."@en .
ns:inPhrasebook a owl:ObjectProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range ns:Phrasebook ; rdfs:label "in phrasebook"@en ; ns:status "stable" .
ns:meterFamily a owl:ObjectProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range ns:MeterFamily ; rdfs:label "meter family"@en ; ns:status "stable" .
ns:feel a owl:ObjectProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range ns:Feel ; rdfs:label "feel"@en ; ns:status "stable" .
ns:meter a owl:DatatypeProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range xsd:string ; rdfs:label "meter"@en ; ns:status "stable" ;
    rdfs:comment "The meter as a person names it: iambic pentameter, common meter."@en .
ns:stressPattern a owl:DatatypeProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range xsd:string ; rdfs:label "stress pattern"@en ; ns:status "stable" ;
    rdfs:comment "The meter as a BeatScript step pattern, one step a syllable: x stressed, ~ not (~ x is an iamb)."@en .
ns:syllableCount a owl:DatatypeProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range xsd:integer ; rdfs:label "syllable count"@en ; ns:status "stable" ;
    rdfs:comment "How many notes the line takes as sung: the steps of its stress pattern."@en .
ns:callSyllables a owl:DatatypeProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range xsd:integer ; rdfs:label "call syllables"@en ; ns:status "stable" ;
    rdfs:comment "For a work song, how many of the first syllables are the leader's call; the rest is the crew's answer."@en .
ns:lyric a owl:DatatypeProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range xsd:string ; rdfs:label "lyric"@en ; ns:status "stable" ;
    rdfs:comment "The words as BeatScript sings them: words of its lexicon as they are, every other word as a phoneme escape."@en .
ns:firstPublished a owl:DatatypeProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range xsd:gYear ; rdfs:label "first published"@en ; ns:status "stable" ;
    rdfs:comment "The year the work was first published (a work song: the year of the earliest collection this wording was read in)."@en .
ns:edition a owl:DatatypeProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range xsd:string ; rdfs:label "edition"@en ; ns:status "stable" ;
    rdfs:comment "The pre-1929 edition the text was read from."@en .
ns:sourceWork a owl:DatatypeProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range xsd:string ; rdfs:label "source work"@en ; ns:status "stable" ;
    rdfs:comment "The work the line is from, and where in it."@en .
ns:sourceAuthor a owl:DatatypeProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:range xsd:string ; rdfs:label "source author"@en ; ns:status "stable" ;
    rdfs:comment "Who wrote the work: a name, or Anonymous."@en .
ns:sourceUrl a owl:ObjectProperty ; rdfs:domain ns:PhrasebookEntry ; rdfs:label "source URL"@en ; ns:status "stable" ;
    rdfs:comment "Where the edition the text was read from can be read online."@en .
"""

FORMS = """
# --- forms and the public-domain theme library ----------------------------------------------------------------

ns:Form a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "form"@en ; ns:status "stable" ;
    rdfs:comment "A shape for a whole track from classical music: its sections, the order they come in, and the plan of arrange mutations that changes them pass after pass. Each is a template built from public-domain themes (docs/forms)."@en .
ns:formTemplate a owl:ObjectProperty ; rdfs:domain ns:Form ; rdfs:range ns:PhraseTemplate ; rdfs:label "form template"@en ; ns:status "stable" ;
    rdfs:comment "The template that writes the form: arrange from template(<name>, <phrase>)."@en .
ns:usesMutation a owl:ObjectProperty ; rdfs:domain ns:Form ; rdfs:range ns:MutationOperator ; rdfs:label "uses mutation"@en ; ns:status "stable" ;
    rdfs:comment "An arrange mutation operator the form's plan uses."@en .
ns:usesTechnique a owl:ObjectProperty ; rdfs:domain ns:Form ; rdfs:range ns:Technique ; rdfs:label "uses technique"@en ; ns:status "stable" ;
    rdfs:comment "A composition technique the form is written with (found phrases: the vocal line)."@en .
ns:usesThemeLibrary a owl:ObjectProperty ; rdfs:range ns:ThemeLibrary ; rdfs:label "uses theme library"@en ; ns:status "stable" ;
    rdfs:comment "The theme library a form or a technique takes its found melodies from."@en .
ns:formShape a owl:DatatypeProperty ; rdfs:domain ns:Form ; rdfs:range xsd:string ; rdfs:label "form shape"@en ; ns:status "stable" ;
    rdfs:comment "The sections of the form and the order they come in."@en .
ns:mutationPlan a owl:DatatypeProperty ; rdfs:domain ns:Form ; rdfs:range xsd:string ; rdfs:label "mutation plan"@en ; ns:status "stable" ;
    rdfs:comment "What the form's arrangement does to its sections, pass on pass."@en .
ns:usesTheme a owl:DatatypeProperty ; rdfs:domain ns:Form ; rdfs:range xsd:string ; rdfs:label "uses theme"@en ; ns:status "stable" ;
    rdfs:comment "The id of a theme of the public-domain theme library the form is built from."@en .
ns:referenceProgram a owl:DatatypeProperty ; rdfs:domain ns:Form ; rdfs:range xsd:string ; rdfs:label "reference program"@en ; ns:status "stable" ;
    rdfs:comment "The CID of a whole hand-written track in the form."@en .
ns:ThemeLibrary a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "theme library"@en ; ns:status "stable" ;
    rdfs:comment "A curated set of melodies, basses and figures written as BeatScript bars to borrow. The public-domain one is music composed and first published before 1929, written from the melody itself, as a CC0 file with one CID: the melodic twin of the phrasebook."@en .
ns:ThemeEntry a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "theme"@en ; ns:status "stable" ;
    rdfs:comment "One theme of a theme library: its bars in a stated key, its role, meter, source work, composer and year, and the themes it pairs with."@en .
ns:ThemeRole a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "theme role"@en ; ns:status "stable" ;
    rdfs:comment "The part a theme is written for: melody, bass, harmony or arp."@en .
ns:ThemeMode a owl:Class ; rdfs:subClassOf ns:MusicConcept ; rdfs:label "theme mode"@en ; ns:status "stable" ;
    rdfs:comment "Whether a theme is in a major key, a minor key, or neither (a pedal or an ostinato)."@en .
ns:inThemeLibrary a owl:ObjectProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range ns:ThemeLibrary ; rdfs:label "in theme library"@en ; ns:status "stable" .
ns:themeRole a owl:ObjectProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range ns:ThemeRole ; rdfs:label "theme role"@en ; ns:status "stable" .
ns:themeMode a owl:ObjectProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range ns:ThemeMode ; rdfs:label "theme mode"@en ; ns:status "stable" .
ns:pairsWith a owl:ObjectProperty, owl:SymmetricProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range ns:ThemeEntry ; rdfs:label "pairs with"@en ; ns:status "stable" ;
    rdfs:comment "A theme that shares this one's key or harmony and morphs well with it (~ a_* morph b_1 0.25)."@en .
ns:timeSignature a owl:DatatypeProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range xsd:string ; rdfs:label "time signature"@en ; ns:status "stable" ;
    rdfs:comment "The meter the theme is written in, as a time signature: 4/4, 2/4, 3/4, 5/4."@en .
ns:cycleSteps a owl:DatatypeProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range xsd:integer ; rdfs:label "cycle steps"@en ; ns:status "stable" ;
    rdfs:comment "The sixteenths one bar of the theme takes on the grid: 16, 12 for 3/4, 20 for 5/4."@en .
ns:barPattern a owl:DatatypeProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range xsd:string ; rdfs:label "bar pattern"@en ; ns:status "stable" ;
    rdfs:comment "One bar of the theme as a BeatScript pattern (the bars in order, as a list)."@en .
ns:letText a owl:DatatypeProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range xsd:string ; rdfs:label "let text"@en ; ns:status "stable" ;
    rdfs:comment "The theme as the let lines a program pastes: a let a bar, then their alternation."@en .
ns:partText a owl:DatatypeProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range xsd:string ; rdfs:label "part text"@en ; ns:status "stable" ;
    rdfs:comment "The part that plays the theme: its role, its cycle and its name (bass/20 mars_ostinato)."@en .
ns:themeWork a owl:DatatypeProperty ; rdfs:domain ns:ThemeEntry ; rdfs:range xsd:string ; rdfs:label "theme work"@en ; ns:status "stable" ;
    rdfs:comment "The work the theme is from, and where in it."@en .
"""

HEADER = """@prefix owl:   <http://www.w3.org/2002/07/owl#> .
@prefix rdfs:  <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:   <http://www.w3.org/2001/XMLSchema#> .
@prefix skos:  <http://www.w3.org/2004/02/skos/core#> .
@prefix ns:    <https://beatscript.app/ontology/ns#> .

# GENERATED by tools/ontology/beatscript.py from beatscript-serve's vocab dump ({source}). Do not edit: change BeatScript's
# grammar, instruments, control registry or MCP tools in BeatScript, then crates/beatscript-serve/ontology/build.sh.
"""


def main():
    src = json.loads(Path(sys.argv[1]).read_text())
    odir = Path(sys.argv[2])
    defined = set()
    # the fork's own hand-written terms
    for m in re.finditer(r"^(concept|instrument):([a-z0-9-]+) a ", (odir / "fork.ttl").read_text(), re.M):
        defined.add(f"<{BASE}{m.group(1)}/{m.group(2)}>")
    # the bitwrap release's typed concept: and instrument: terms, under the fork's base (where the build re-homes them)
    up = re.compile(r"^<" + re.escape(UPSTREAM_BASE) + r"((?:concept|instrument)/[a-z0-9-]+)> <" + re.escape(RDF_TYPE) + "> ")
    for line in (odir / "upstream" / "bitwrap-0.2.0" / "ns.nt").read_text().splitlines():
        m = up.match(line)
        if m:
            defined.add(f"<{BASE}{m.group(1)}>")
    out = Out(defined)
    c = lambda s: iri("concept/" + s)
    skos_concept = f"<{SKOS}Concept>"

    # the grammar
    prods = productions(src["grammar_ebnf"])
    names = {p[0] for p in prods}
    seen_p = {}
    for p in prods:
        k = production_slug(p[0])
        if k in seen_p:
            sys.exit(f"grammar: productions {seen_p[k]!r} and {p[0]!r} would share the term {k}")
        seen_p[k] = p[0]
    keywords, tokens = {}, {}
    for name, rhs, section, notes in prods:
        terms, refs = rhs_symbols(rhs)
        facts = [(nsx("ebnf"), lit(rhs, None)), (nsx("writtenAs"), lit(name, None))]
        if section:
            facts.append((nsx("grammarSection"), lit(section)))
        if any("[host]" in n for n in notes):
            facts.append((nsx("hostOnly"), "true"))
        for r in sorted(set(refs)):
            if r in names:
                facts.append((nsx("usesProduction"), c(production_slug(r))))
            elif r.isupper():
                tokens.setdefault(r, set()).add(name)
                facts.append((nsx("usesToken"), c("token-" + slug(r))))
        for t in sorted(set(terms)):
            keywords.setdefault(t, set()).add(name)
            facts.append((nsx("usesKeyword"), c("keyword-" + keyword_slug(t))))
        said = "; ".join(x for x in (n.replace("[host]", "").strip() for n in notes) if x)
        definition = f"The {name} production of BeatScript's grammar" + (f" ({said})." if said else ".")
        out.term(c(production_slug(name)), [skos_concept, nsx("GrammarProduction")], name, definition, facts, "GrammarProduction")
    seen_kw = {}
    for t in sorted(keywords):
        s = keyword_slug(t)
        if s in seen_kw:
            sys.exit(f"grammar: terminals {seen_kw[s]!r} and {t!r} would share the term keyword-{s}")
        seen_kw[s] = t
        used = ", ".join(sorted(keywords[t]))
        out.term(c("keyword-" + s), [skos_concept, nsx("Keyword")], t, f"The keyword {t} of BeatScript, written in: {used}.", [(nsx("keywordText"), lit(t, None))], "Keyword")
    for t in sorted(tokens):
        out.term(c("token-" + slug(t)), [skos_concept, nsx("TokenClass")], t, f"The lexer's {t} token, used in: {', '.join(sorted(tokens[t]))}.", [(nsx("writtenAs"), lit(t, None))], "TokenClass")
    prod_terms = {(p[0], t) for p in prods for t in rhs_symbols(p[1])[0]}
    for prod, term, s, label, definition in MEANINGS:
        if (prod, term) in prod_terms:
            kind = "MutationOperator" if prod == "mutation" else "PatternModifier"
            types = [skos_concept, nsx("PatternModifier")] if kind == "PatternModifier" else [skos_concept, nsx("LanguageTerm")]
            out.term(c(s), types, label, definition, [(nsx("markedBy"), c("keyword-" + keyword_slug(term))), (nsx("writtenAs"), lit(term, None))], kind if kind == "PatternModifier" else "MutationSigil")

    # the count after a mutation's mark (~3, @4), emitted while the `mutation` production takes an INT
    if any(p[0] == "mutation" and "INT" in rhs_symbols(p[1])[1] for p in prods):
        out.term(c("mutation-every-nth-pass"), [skos_concept, nsx("LanguageTerm")], "mutation count (~N, @N)", "An arrange mutation written ~N target op (or @N target op) fires only on every Nth pass of its arrange block, on passes N, 2N, 3N and so on counted from 1; with no count it fires on every pass.", [(nsx("markedBy"), c("token-int")), (nsx("writtenAs"), lit("~N", None))], "MutationSigil")

    # the language's versions, oldest first: each a revision of the one before it
    versions = src.get("versions", [])
    language = c("product-beatscript-language")
    prev = None
    for v in versions:
        n, line, head = v["number"], v["line"], v["canonical_head"]
        definition = (f"BeatScript {n}: a text is in it when its header block has the version line {line}" + (", or no version line at all" if v.get("default") else "")
                      + f". Its canonical text {'begins with ' + head.rstrip() if head else 'has no #! line'}, and its program CID is the CID of the document "
                      + json.dumps({"format": v["cid_format"], "text": "<canonical text>", "type": v["cid_type"]}, sort_keys=True, separators=(",", ":"))
                      + "." + (" The current version: what fmt upgrades a program to and what the generators write." if v["current"] else ""))
        facts = [(nsx("versionOf"), language), (nsx("versionLine"), lit(line, None)), (nsx("programCidFormat"), typed(v["cid_format"], "integer")),
                 (nsx("programCidType"), lit(v["cid_type"], None))]
        if prev:
            facts.append(("<http://www.w3.org/ns/prov#wasRevisionOf>", prev))
        prev = c(f"beatscript-{n}")
        out.term(prev, [skos_concept, nsx("LanguageVersion")], f"BeatScript {n}", definition, facts, "LanguageVersion")
    for v in versions:
        if v["current"]:
            out.facts(language, [(nsx("currentVersion"), c(f"beatscript-{v['number']}"))])

    # language registries
    lang = src["language"]
    for g in lang["generators"] + [x for x in ("choose", "deg") if x in keywords]:
        facts = [(nsx("writtenAs"), lit(g, None))]
        if g in GENERATOR_SEE_ALSO:
            facts.append(("rdfs:seeAlso", nsx(GENERATOR_SEE_ALSO[g])))
        out.term(c("generator-" + slug(g)), [skos_concept, nsx("PatternGenerator")], g, f"The pattern generator {g}: a call that makes a pattern from its arguments.", facts, "PatternGenerator")
    for t in lang["pattern_transforms"]:
        out.term(c("transform-" + slug(t)), [skos_concept, nsx("PatternTransform")], t, f"The pattern call {t}: takes a pattern and gives another.", [(nsx("writtenAs"), lit(t, None))], "PatternTransform")
    for t in lang["evolve_ops"]:
        out.term(c("evolve-" + slug(t)), [skos_concept, nsx("EvolveOperator")], t, f"The evolve operator {t}: applied to a part once a generation.", [(nsx("writtenAs"), lit(t, None))], "EvolveOperator")
    for t in lang["mutation_ops"]:
        out.term(c("mutation-op-" + slug(t)), [skos_concept, nsx("MutationOperator")], t, f"The arrange mutation operator {t}.", [(nsx("writtenAs"), lit(t, None))], "MutationOperator")
    for t in lang["templates"]:
        out.term(c("template-" + slug(t["name"])), [skos_concept, nsx("PhraseTemplate")], t["name"], f"The phrase template {t['name']}: {t['summary']}. Written {t['example']}.", [(nsx("writtenAs"), lit(t["name"], None)), ("rdfs:seeAlso", c(production_slug("template")))], "PhraseTemplate")

    # instruments
    inst = src["instruments"]
    for f in inst["families"]:
        out.term(c("voice-family-" + slug(f)), [skos_concept, nsx("VoiceFamily")], f, f"The {f} family: an inst= name containing {f} plays its generic voice.", [(nsx("writtenAs"), lit(f, None))], "VoiceFamily")
    fams = sorted(inst["families"], key=len, reverse=True)
    for v in inst["voices"]:
        facts = [(nsx("writtenAs"), lit(f"inst={v}", None))]
        for f in inst["families"]:
            if f in v.split("-") or v == f:
                facts.append((nsx("inVoiceFamily"), c("voice-family-" + slug(f))))
        out.term(iri("instrument/" + slug(v)), [nsx("BeatScriptVoice")], v, f"The BeatScript voice {v}, written inst={v}.", facts, "BeatScriptVoice")
    for k in inst["kits"]:
        out.term(iri("instrument/" + slug(k)), [nsx("DrumKit")], k, f"The drum kit {k}, written kit={k}.", [(nsx("writtenAs"), lit(f"kit={k}", None))], "DrumKit")
    for p in inst["palettes"]:
        out.term(c("palette-" + slug(p)), [skos_concept, nsx("Palette")], p, f"The {p} palette.", [(nsx("writtenAs"), lit(p, None))], "Palette")
    del fams

    # the public-domain phrasebook: the corpus, its meter families and feels (closed sets in djengine; the lines are data)
    pb = src.get("phrasebook")
    if pb:
        out.term(c("phrasebook-public-domain"), [skos_concept, nsx("Phrasebook")], "public-domain phrasebook",
                 "Lines of verse, scripture, speech and work song first published before 1929, each with its source, its meter as a step pattern and the lyric BeatScript sings it as. A CC0 file, every text in the public domain, addressed by the CID of its text (beatscript.app/phrasebook).",
                 [(nsx("writtenAs"), lit("phrasebook-pd", None)), (f"<http://purl.org/dc/terms/license>", "<https://creativecommons.org/publicdomain/zero/1.0/>")], "Phrasebook")
        for f in pb["families"]:
            facts = [(nsx("writtenAs"), lit(f["id"], None))]
            if f.get("groove"):
                facts.append((nsx("suitsGroove"), lit(f["groove"], None)))
            facts += [(nsx("suitsGenre"), iri("genre/" + g)) for g in f.get("genres", [])]
            out.term(c("phrasebook-family-" + slug(f["id"])), [skos_concept, nsx("MeterFamily")], f["name"], f["about"], facts, "MeterFamily")
        out.term(c("technique-found-phrases"), [skos_concept, nsx("Technique")], "found phrases",
                 "Borrowing found material for its shape, not its meaning: a public-domain line is chosen for its meter (trochees chant on the kick, iambs lilt on the offbeat, anapests and dactyls want a triplet feel, shanties call and respond, psalm parallelism repeats with variation), its stresses are written to land where the meter says, the vocal is kept low (about A3 to F4) and cycled between a buried section and a spotlit one, crossed onto the drum rhythms with xfer, and its pronunciation fixed with phoneme escapes. The lineage of sampling, cut-ups and readymades, borrowing structure rather than recordings, from sources clean by construction.",
                 [("rdfs:seeAlso", "<https://beatscript.app/docs/found-phrases>"), (nsx("usesPhrasebook"), c("phrasebook-public-domain")), (nsx("writtenAs"), lit("found-phrases", None))], "Technique")
        for f in pb["feels"]:
            out.term(c("phrasebook-feel-" + slug(f["id"])), [skos_concept, nsx("Feel")], f["id"], f"A {f['id']} feel: {f['about']}.", [(nsx("writtenAs"), lit(f["id"], None))], "Feel")

    # forms (template.rs `Form`) and the public-domain theme library's closed vocabularies (the themes themselves are data)
    th = src.get("themes")
    forms = lang.get("forms", [])
    if th:
        out.term(c("themes-public-domain"), [skos_concept, nsx("ThemeLibrary")], "public-domain theme library",
                 "Melodies, basses and figures composed and first published before 1929, each written from the melody itself as BeatScript bars in a stated key, with its role, meter, source work, composer, year and pairings. A CC0 file, addressed by the CID of its text (beatscript.app/themes): the melodic twin of the phrasebook.",
                 [(nsx("writtenAs"), lit("themes-pd", None)), (f"<http://purl.org/dc/terms/license>", "<https://creativecommons.org/publicdomain/zero/1.0/>"), ("rdfs:seeAlso", "<https://beatscript.app/themes>")], "ThemeLibrary")
        for r in th["roles"]:
            out.term(c("theme-role-" + slug(r["id"])), [skos_concept, nsx("ThemeRole")], r["id"], f"A theme written for the {r['id']} part: {r['about']}.", [(nsx("writtenAs"), lit(r["id"], None))], "ThemeRole")
        for m in th["modes"]:
            out.term(c("theme-mode-" + slug(m["id"])), [skos_concept, nsx("ThemeMode")], m["id"], f"A theme {m['about']}.", [(nsx("writtenAs"), lit(m["id"], None))], "ThemeMode")
    for f in forms:
        facts = [(nsx("writtenAs"), lit(f["name"], None)), (nsx("formTemplate"), c("template-" + slug(f["name"]))), (nsx("formShape"), lit(f["shape"])), (nsx("mutationPlan"), lit(f["plan"])),
                 ("rdfs:seeAlso", f"<https://beatscript.app/docs/forms#{f['name']}>")]
        if pb:
            facts.append((nsx("usesTechnique"), c("technique-found-phrases")))
        if th:
            facts.append((nsx("usesThemeLibrary"), c("themes-public-domain")))
        facts += [(nsx("usesMutation"), c("mutation-op-" + slug(m))) for m in f["mutations"]]
        facts += [(nsx("usesTheme"), lit(t, None)) for t in f["themes"]]
        if f.get("reference"):
            facts.append((nsx("referenceProgram"), lit(f["reference"]["cid"], None)))
        out.term(c("form-" + slug(f["name"])), [skos_concept, nsx("Form")], f["label"], f"The {f['label']}, a form from classical music: {f['shape']}. {f['plan'][0].upper() + f['plan'][1:]}.", facts, "Form")

    # the control registry
    sugar = {}
    for word, noun in lang["sugar"]:
        sugar.setdefault(noun, []).append(word)
    verbs = {}
    for n in src["control_nouns"]:
        name = n["name"]
        facts = [(nsx("writtenAs"), lit(f"do {name}", None))]
        for a in n["actions"]:
            verbs.setdefault(a["name"], set()).add(name)
            facts.append((nsx("acceptsVerb"), c("verb-" + slug(a["name"]))))
        if n.get("default_action"):
            facts.append((nsx("defaultVerb"), c("verb-" + slug(n["default_action"]))))
        for p in n.get("params", []):
            facts.append((nsx("controlParameter"), lit(p["name"], None)))
        for w in sorted(sugar.get(name, [])):
            facts.append((nsx("sugarWord"), lit(w, None)))
        out.term(c("noun-" + slug(name)), [skos_concept, nsx("ControlNoun")], name, f"The control noun {name}: do {name} <verb>.", facts, "ControlNoun")
    for v in sorted(verbs):
        facts = [(nsx("actsOn"), c("noun-" + slug(n))) for n in sorted(verbs[v])]
        out.term(c("verb-" + slug(v)), [skos_concept, nsx("ControlVerb")], v, f"The control verb {v}, accepted by: {', '.join(sorted(verbs[v]))}.", facts, "ControlVerb")

    # the MCP tools
    servers = [("beatscript", c("product-beatscript-mcp"), src["mcp"]["beatscript"], set(src["mcp"].get("beatscript_auth", []))),
               ("ontology", c("product-beatscript-ontology-mcp"), src["mcp"]["ontology"], set())]
    for key, server, tools, auth in servers:
        for t in tools:
            ann = t.get("annotations", {})
            schema = json.dumps(t["inputSchema"], sort_keys=True, separators=(",", ":"), ensure_ascii=False)
            props = sorted(t["inputSchema"].get("properties", {}))
            req = sorted(t["inputSchema"].get("required", []))
            ro = ann.get("readOnlyHint", key == "ontology")
            facts = [(nsx("providedBy"), server), (nsx("toolName"), lit(t["name"], None)), (nsx("inputSchema"), lit(schema, None)),
                     (nsx("readOnly"), "true" if ro else "false"), (nsx("needsToken"), "true" if t["name"] in auth else "false")]
            if ann.get("title"):
                facts.append((nsx("toolTitle"), lit(ann["title"])))
            facts += [(nsx("toolParameter"), lit(p, None)) for p in props if p not in req]
            facts += [(nsx("requiredParameter"), lit(p, None)) for p in req]
            out.term(c(f"tool-{key}-{slug(t['name'])}"), [nsx("McpTool")], t["name"], t["description"], facts, f"McpTool:{key}")
    ont_mcp = f"""{c("product-beatscript-ontology-mcp")} a skos:Concept, ns:McpServer ; skos:prefLabel "BeatScript ontology MCP server"@en ; ns:status "stable" ;
    skos:definition {lit(src["mcp"].get("ontology_instructions", "Read-only access to the BeatScript ontology."))} ;
    ns:inProductFamily {c("product-beatscript")} ; ns:homepage "https://beatscript.app/ontology/"^^xsd:anyURI ."""

    # phonemes (CC0, once the set lands)
    ph = src.get("phonemes")
    extra = ""
    if ph:
        set_iri = c("phoneme-set-" + slug(ph["name"]))
        extra = f"""
ns:PhonemeSet a owl:Class ; rdfs:subClassOf ns:DjConcept ; rdfs:label "phoneme set"@en ; ns:status "stable" ;
    rdfs:comment "A set of phoneme symbols a vocal line is spelled in. The public set is BeatScript's own, CC0."@en .
ns:Phoneme a owl:Class ; rdfs:subClassOf ns:DjConcept ; rdfs:label "phoneme"@en ; ns:status "stable" ;
    rdfs:comment "One sound of a phoneme set, with its sound class and its IPA and ARPAbet equivalents as data."@en .
ns:inPhonemeSet a owl:ObjectProperty ; rdfs:domain ns:Phoneme ; rdfs:range ns:PhonemeSet ; rdfs:label "in phoneme set"@en ; ns:status "stable" .
ns:usesPhonemeSet a owl:ObjectProperty ; rdfs:domain ns:VocalLine ; rdfs:range ns:PhonemeSet ; rdfs:label "uses phoneme set"@en ; ns:status "stable" .
ns:soundClass a owl:DatatypeProperty ; rdfs:domain ns:Phoneme ; rdfs:range xsd:string ; rdfs:label "sound class"@en ; ns:status "stable" .
ns:ipa a owl:DatatypeProperty ; rdfs:domain ns:Phoneme ; rdfs:range xsd:string ; rdfs:label "IPA"@en ; ns:status "stable" .
ns:arpabet a owl:DatatypeProperty ; rdfs:domain ns:Phoneme ; rdfs:range xsd:string ; rdfs:label "ARPAbet"@en ; ns:status "stable" .
{set_iri} a skos:Concept, ns:PhonemeSet ; skos:prefLabel {lit(ph["name"])} ; ns:status "stable" ;
    skos:definition {lit("BeatScript's own phoneme symbols (version " + str(ph.get("version", 1)) + "), the ones a vocal line is spelled in on every public build. CC0, written for BeatScript: no third-party pronouncing dictionary is part of it.")} ;
    <http://purl.org/dc/terms/license> <https://creativecommons.org/publicdomain/zero/1.0/> .
ns:VocalLine rdfs:seeAlso ns:PhonemeSet .
"""
        rest = [{"symbol": ph["rest"], "class": "rest", "definition": "A rest: no sound, the time a syllable is silent."}] if ph.get("rest") else []
        for p in ph["phonemes"] + rest:
            facts = [(nsx("inPhonemeSet"), set_iri), (nsx("writtenAs"), lit(p["symbol"], None)), (nsx("soundClass"), lit(p["class"], None))]
            for ipa in [x.strip() for x in p.get("ipa", "").split(",") if x.strip()]:
                facts.append((nsx("ipa"), lit(ipa, None)))
            if p.get("arpabet"):
                facts.append((nsx("arpabet"), lit(p["arpabet"], None)))
            if p.get("example"):
                facts.append((f"<{SKOS}example>", lit(p["example"])))
            out.term(c("phoneme-" + slug(ph["name"]) + "-" + (slug(p["symbol"]) if p["symbol"] != "_" else "rest")), [skos_concept, nsx("Phoneme")], p["symbol"], p.get("definition") or f"The phoneme {p['symbol']} ({p['class']}).", facts, "Phoneme")

    source = src.get("source", {})
    if versions:
        extra = VERSIONS + extra
    if pb:
        extra += PHRASEBOOK
    if src.get("themes") or lang.get("forms"):
        extra += FORMS
    hdr = HEADER.format(source=f"{source.get('repo', '?')} {source.get('commit', '?')}") + CLASSES + extra + "\n" + ont_mcp + "\n\n"
    sys.stdout.write(out.text(hdr))
    sys.stderr.write(json.dumps(dict(sorted(out.count.items()))) + "\n")


if __name__ == "__main__":
    main()
