#!/usr/bin/env python3
"""Record the ontology MCP's answers from a running server, for src/ontology/tests.rs (mcp_tools_match_fixtures).

    python3 -I ontology/fixtures.py [URL]     default http://127.0.0.1:18098/ontology/mcp (a local beatscript-serve)

Writes ontology/fixtures/mcp-<version>.json: one entry per call, {"call", "text"} or {"call", "error"}, plus
"unordered": true where the query has no ORDER BY (row order is the engine's, not part of the answer). Record against
a server that serves the release in ontology/release/ (the version is read from it and checked): up to bitwrap 0.2.0
that was ontology.bitwrap.io, the Go server the port matches; for the BeatScript ontology it is beatscript-serve itself
(`cargo run -p beatscript-serve -- -addr 127.0.0.1:18098 -data <tmp>`), so the fixtures pin the answers against unintended change.
"""
import json
import os
import sys
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
URL = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:18098/ontology/mcp"
NS = "https://beatscript.app/ontology/ns#"
BASE = "https://beatscript.app/ontology/"

CALLS = [
    ("get_release", {}),
    ("list_classes", {}),
    ("list_genres", {}),
    ("list_genres", {"family": "edm"}),
    ("list_genres", {"min_bpm": 100, "max_bpm": 130}),
    ("get_genre", {"genre": "techno"}),
    ("get_genre", {"genre": "jazz"}),
    ("get_genre", {"genre": "Lofi "}),
    ("get_genre", {"genre": "polka"}),
    ("describe_term", {"term": "Genre"}),
    ("describe_term", {"term": "genre:techno"}),
    ("describe_term", {"term": "bpm"}),
    ("describe_term", {"term": "Genre", "format": "json"}),
    ("describe_term", {"term": "song section", "format": "json"}),
    ("describe_term", {"term": "concept:bjorklund", "format": "json"}),
    ("describe_term", {"term": "groove:four-on-floor", "format": "json"}),
    ("describe_term", {"term": "https://example.com/x"}),
    ("describe_term", {"term": "nosuchthing"}),
    ("search_terms", {"text": "euclid"}),
    ("search_terms", {"text": "drop", "limit": 5}),
    ("search_terms", {"text": "inhibitor", "limit": 10}),
    ("sparql_query", {"query": "SELECT ?g ?bpm WHERE { ?g a <%sGenre> ; <%sbpm> ?bpm } ORDER BY ?bpm ?g" % (NS, NS)}),
    ("sparql_query", {"query": "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }"}),
    ("sparql_query", {"query": "ASK { <%sgenre/techno> a <%sGenre> }" % (BASE, NS)}),
    ("sparql_query", {"query": "SELECT ?up WHERE { <%sgenre/techno> <http://www.w3.org/ns/prov#wasDerivedFrom> ?up }" % BASE}),
    ("sparql_query", {"query": "SELECT ?p ?o WHERE { <%sns> ?p ?o FILTER(isIRI(?o)) } ORDER BY ?p ?o" % BASE}),
    ("describe_term", {"term": "https://ontology.bitwrap.io/genre/techno"}),
    ("sparql_query", {"query": "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\nSELECT ?c WHERE { ?c rdfs:subClassOf <%sPetriNet> }" % NS}, True),
    ("sparql_query", {"query": "CONSTRUCT { ?g <%sbpm> ?b } WHERE { ?g <%sbpm> ?b } " % (NS, NS)}),
    ("sparql_query", {"query": "SELECT * WHERE { SERVICE <http://example.com/sparql> { ?s ?p ?o } }"}),
    ("sparql_query", {"query": "INSERT DATA { <a:a> <a:b> <a:c> }"}),
]

# The generated BeatScript vocabulary (bitwrap's 0.3.0 had it; the BeatScript ontology has it from its 0.1.0).
CALLS_VOCAB = [
    ("search_terms", {"text": "BeatScript", "limit": 30}),
    ("describe_term", {"term": "concept:grammar-term", "format": "json"}),
    ("describe_term", {"term": "concept:grammar-arrange-block"}),
    ("describe_term", {"term": "concept:tool-beatscript-check", "format": "json"}),
    ("describe_term", {"term": "concept:modifier-slide", "format": "json"}),
    ("describe_term", {"term": "instrument:808-bass", "format": "json"}),
    ("describe_term", {"term": "McpTool", "format": "json"}),
    ("list_classes", {}),
    ("sparql_query", {"query": "SELECT ?n ?ro ?tok WHERE { ?t a <%sMcpTool> ; <%stoolName> ?n ; <%sreadOnly> ?ro ; <%sneedsToken> ?tok } ORDER BY ?n ?ro" % (NS, NS, NS, NS)}),
    ("sparql_query", {"query": "SELECT ?c (COUNT(?x) AS ?n) WHERE { VALUES ?c { <%sGrammarProduction> <%sKeyword> <%sBeatScriptVoice> <%sControlNoun> <%sControlVerb> <%sMutationOperator> } ?x a ?c } GROUP BY ?c ORDER BY ?c" % ((NS,) * 6)}),
]

# BeatScript 2 (the BeatScript ontology's 0.2.0): the language's versions, and the name btw kept as a former name.
CALLS_VERSIONS = [
    ("describe_term", {"term": "concept:beatscript-2", "format": "json"}),
    ("describe_term", {"term": "concept:beatscript-1"}),
    ("describe_term", {"term": "concept:product-beatscript-language", "format": "json"}),
    ("describe_term", {"term": "Program", "format": "json"}),
    ("search_terms", {"text": "btw", "limit": 30}),
    ("sparql_query", {"query": "SELECT ?v ?line ?format ?type ?revises WHERE { ?v a <%sLanguageVersion> ; <%sversionLine> ?line ; <%sprogramCidFormat> ?format ; <%sprogramCidType> ?type OPTIONAL { ?v <http://www.w3.org/ns/prov#wasRevisionOf> ?revises } } ORDER BY ?v" % ((NS,) * 4)}),
    ("sparql_query", {"query": "SELECT ?p ?cid ?version WHERE { ?p a <%sProgram> ; <%sprogramCid> ?cid OPTIONAL { ?p <%sinLanguageVersion> ?version } } ORDER BY ?p" % ((NS,) * 3)}),
]


calls_made = []


def rpc(body):
    req = urllib.request.Request(URL, data=json.dumps(body).encode(), headers={"Content-Type": "application/json", "User-Agent": "beatscript-serve-fixtures", "X-Real-IP": f"198.51.100.{len(calls_made) % 250 + 1}"})
    calls_made.append(1)
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def main():
    vendored = json.load(open(os.path.join(HERE, "release", "manifest.json")))["version"]
    live = json.loads(rpc({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "get_release", "arguments": {}}})["result"]["content"][0]["text"])["version"]
    if live != vendored:
        sys.exit(f"{URL} serves {live}, ontology/release/ is {vendored}: record against the server for the vendored release")
    out = []
    calls = CALLS + CALLS_VOCAB + CALLS_VERSIONS
    for c in calls:
        name, args = c[0], c[1]
        for _ in range(20):
            r = rpc({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})
            if "result" in r or r.get("error", {}).get("code") != -32000:
                break
            time.sleep(3.5)  # rate limited: one call back every 3 s
        res = r["result"]
        e = {"call": {"name": name, "arguments": args}}
        text = res["content"][0]["text"]
        if res.get("isError"):
            e["error"] = text
        else:
            e["text"] = text
        if len(c) > 2 and c[2]:
            e["unordered"] = True
        out.append(e)
        time.sleep(0 if "127.0.0.1" in URL else 1.2)  # a public server's bucket: 20 calls, one back every 3 s
    os.makedirs(os.path.join(HERE, "fixtures"), exist_ok=True)
    path = os.path.join(HERE, "fixtures", f"mcp-{vendored}.json")
    with open(path, "w") as f:
        json.dump(out, f, indent=1, ensure_ascii=False)
        f.write("\n")
    print(f"{path}: {len(out)} calls from {URL}")


if __name__ == "__main__":
    main()
