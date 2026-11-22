//! GENERATED once from the hand-written MCP tool table (djctl `app_mcp`) and kept by hand
//! from here: the registry's data. See `super` for what each field means.

use super::{Action, Effect, Noun, Param};

const A_AUDIO: &[Action] = &[
    Action::new("devices", "every device, the output, each strip", Effect::Read),
    Action::new("output", "switch the output device", Effect::Live).probe(r#"{"device":"__no_such_device__"}"#),
    Action::new("input", "route an input strip or pick its channel", Effect::Live).probe(r#"{"input":"__no_such_strip__"}"#),
];

const A_DECK: &[Action] = &[
    Action::new("state", "the deck's state", Effect::Read),
    Action::new("play", "play (on=false pauses)", Effect::Write),
    Action::new("pause", "pause", Effect::Write),
    Action::new("toggle", "play / pause", Effect::Write),
    Action::new("stop", "stop", Effect::Write),
    Action::new("cue", "cue", Effect::Write),
    Action::new("load", "load a generated track (genre, seed, structure | spec)", Effect::Write),
    Action::new("demo", "the demo loop", Effect::Write),
    Action::new("file", "an audio file", Effect::Write),
    Action::new("next", "next track", Effect::Write),
    Action::new("seed_step", "seed ± delta", Effect::Write),
    Action::new("tempo", "set bpm", Effect::Write),
    Action::new("pitch", "pitch fader", Effect::Write),
    Action::new("pitch_range", "pitch fader travel", Effect::Write),
    Action::new("nudge", "nudge the tempo", Effect::Write),
    Action::new("sync", "sync to the other deck (beat | bar)", Effect::Write),
    Action::new("section", "jump to a section", Effect::Write),
    Action::new("hot_cue", "set / jump to a hot cue", Effect::Write),
    Action::new("clear_cue", "clear a hot cue", Effect::Write),
    Action::new("loop", "loop in / out / halve / double / exit / beats", Effect::Write),
    Action::new("seek", "seek in a sample deck", Effect::Write),
    Action::new("role", "switch a role", Effect::Write),
    Action::new("macro", "fire a net macro", Effect::Write),
    Action::new("master_macro", "fire a master macro", Effect::Write),
    Action::new("panic", "end everything running on the deck", Effect::Write),
    Action::new("rewind", "to the start", Effect::Write),
    Action::new("focus", "focus the deck", Effect::Write),
    Action::new("sound", "sound palette: list, set, step", Effect::Write),
    Action::new("auto_gain", "the Auto level chip (on; omitted flips it)", Effect::Write),
    Action::new("role_disable", "lock a role out of the track (role, on; omitted flips it)", Effect::Write)
        .probe(r#"{"role":"__no_such_role__"}"#),
    Action::new("copy", "copy the other deck onto this one", Effect::Write),
];

const A_TRANSPORT: &[Action] = &[
    Action::new("state", "read", Effect::Read),
    Action::new("start", "start the clock", Effect::Write),
    Action::new("stop", "stop the clock", Effect::Write),
    Action::new("toggle", "start / stop", Effect::Write),
    Action::new("tap", "tap tempo", Effect::Write),
    Action::new("lock", "lock the master tempo to a deck", Effect::Write),
    Action::new("unlock", "unlock", Effect::Write),
    Action::new("free_bpm", "tempo when no deck plays", Effect::Write),
    Action::new("auto", "auto transport on / off / toggle", Effect::Write),
];

const A_FX: &[Action] = &[
    Action::new("set", "set an FX param", Effect::Write),
    Action::new("kill", "kill switch", Effect::Write),
    Action::new("reset", "reset the unit", Effect::Write),
    Action::new("params", "list the param ids", Effect::Read),
];

const A_LOOPS: &[Action] = &[
    Action::new("list", "the loop row", Effect::Read),
    Action::new("removed", "the Removed menu: loops taken off the row, this run's and earlier runs'", Effect::Read),
    Action::new("restore_removed", "put a removed loop back where it was (id)", Effect::Write)
        .probe(r#"{"id":"__no_such_loop__"}"#),
    Action::new("delete_removed", "delete a removed loop for good, its audio too (id)", Effect::Live)
        .probe(r#"{"id":"__no_such_loop__"}"#),
    Action::new("root", "loop root: status / set / detect / auto", Effect::Write),
    Action::new("to_deck", "a row clip onto a deck", Effect::Write),
    Action::new("library_to_deck", "a library loop onto a deck", Effect::Write),
    Action::new("play", "play a clip", Effect::Write),
    Action::new("stop", "stop a clip", Effect::Write),
    Action::new("stop_all", "stop every clip", Effect::Write),
    Action::new("remove", "take a clip off the row", Effect::Write),
    Action::new("record", "hold LOOP on a source", Effect::Write),
    Action::new("grab", "take the last bars of a source", Effect::Write),
    Action::new("volume", "the row's volume", Effect::Write),
    Action::new("mute", "mute the row", Effect::Write),
    Action::new("library", "the saved loops", Effect::Read),
    Action::new("library_load", "library loop onto the row", Effect::Write),
    Action::new("library_save", "save a clip to the library", Effect::Live).probe(r#"{"id":"__no_such_clip__"}"#),
    Action::new("library_delete", "delete from the library", Effect::Live).probe(r#"{"id":"__no_such_loop__"}"#),
    Action::new("library_row", "the whole library onto the row", Effect::Write),
];

const A_KEYS: &[Action] = &[
    Action::new("status", "mode, deck, quant, held keys", Effect::Read),
    Action::new("mode", "live / transpose / td3 / toggle", Effect::Write),
    Action::new("deck", "which deck", Effect::Write),
    Action::new("quant", "when keys sound", Effect::Write),
    Action::new("scale_lock", "scale lock", Effect::Write),
    Action::new("note", "play one key", Effect::Write),
    Action::new("off", "let go of every key", Effect::Write),
];

const A_PADS: &[Action] = &[
    Action::new("status", "the mode seen and notes in force", Effect::Read),
    Action::new("learn", "learn the 16 pad notes", Effect::Write),
    Action::new("cancel", "cancel a learn", Effect::Write),
    Action::new("clear", "forget what was learned", Effect::Write),
    Action::new("set", "set channel and notes", Effect::Write),
    Action::new("feed", "feed a MIDI message", Effect::Write),
];

const A_VOX: &[Action] = &[
    Action::new("status", "folder, page, pads", Effect::Read),
    Action::new("dir", "the folder", Effect::Write),
    Action::new("scan", "rescan", Effect::Write),
    Action::new("page", "next / prev / first", Effect::Write),
    Action::new("set", "choke, stretch, quant, bpm", Effect::Write),
    Action::new("play", "play a pad", Effect::Write),
];

const A_CAPTURE: &[Action] = &[
    Action::new("status", "read", Effect::Read),
    Action::new("grab", "the last beats of an input", Effect::Write),
    Action::new("start", "a live take", Effect::Write),
    Action::new("end", "end the take", Effect::Write),
    Action::new("arm", "arm", Effect::Write),
    Action::new("bars", "a loop of exactly N bars cut on bar lines of the input's grid, from the history head", Effect::Write).with(&[
        Param::new("bars", "integer", "1..16 (default 4)"),
        Param::new("anchor", "string", "start: the bars from the head on; end: the bars before it (default start when scrubbed, end when live)"),
    ]),
];

const A_HISTORY: &[Action] = &[
    Action::new("status", "read", Effect::Read),
    Action::new("touch", "hold the head", Effect::Write),
    Action::new("scrub", "scrub by beats or seconds", Effect::Write),
    Action::new("release", "let go", Effect::Write),
    Action::new("live", "back to live", Effect::Write),
    Action::new("loop", "loop the last beats", Effect::Write),
    Action::new("loop_step", "double / halve the loop", Effect::Write),
    Action::new("beats", "the window length", Effect::Write),
    Action::new("jog", "aim a deck's jog", Effect::Write),
    Action::new("snap", "jog's history back to live", Effect::Write),
    Action::new("grid", "the input's beat grid (tempo, bar 1, confidence) and the bar lines in its history window", Effect::Read),
    Action::new("align", "put the head on a line of the grid: the nearest bar (or beat), or steps lines on from it", Effect::Write).with(&[
        Param::new("unit", "string", "bar | beat (default the analysis snap setting, else bar)"),
        Param::new("steps", "integer", "lines to move from the nearest (negative: back in time)"),
    ]),
    Action::new("loop_bars", "loop exactly N bars of the history in place: the head snaps to a bar line and the loop is the bars before it", Effect::Write)
        .with(&[Param::new("bars", "integer", "1..16 (default 1)")]),
];

const A_TD3: &[Action] = &[
    Action::new("status", "read", Effect::Read),
    Action::new("keys", "MPK keys play the TD-3", Effect::Read),
    Action::new("octave", "keys octave", Effect::Write),
    Action::new("channel", "MIDI channel", Effect::Write),
    Action::new("generate", "a new acid pattern", Effect::Write),
    Action::new("slot", "the target slot", Effect::Write),
    Action::new("step", "edit a step / the active count", Effect::Write),
    Action::new("audition", "play one step", Effect::Live).probe(r#"{"step":99}"#),
    Action::new("write", "send the pattern (dry run unless confirm)", Effect::Live).probe(r#"{}"#),
    Action::new("cutoff_cc", "the CC a cutoff trick rides", Effect::Write),
];

const A_KITS: &[Action] = &[
    Action::new("status", "", Effect::Read),
    Action::new("list", "", Effect::Read),
    Action::new("get", "", Effect::Read),
    Action::new("add", "", Effect::Write),
    Action::new("remove", "", Effect::Write),
    Action::new("rename", "", Effect::Write),
    Action::new("duplicate", "", Effect::Write),
    Action::new("select", "", Effect::Write),
    Action::new("host", "", Effect::Write),
    Action::new("pad", "", Effect::Write),
    Action::new("pattern", "", Effect::Write),
    Action::new("build", "", Effect::Live).probe(r#"{"kit":"__no_such_kit__"}"#),
    Action::new("install", "", Effect::Live).probe(r#"{"kit":"__no_such_kit__"}"#),
    Action::new("loaded", "", Effect::Write),
    Action::new("mark_loaded", "", Effect::Write),
    Action::new("forget", "", Effect::Write),
    Action::new("track", "", Effect::Write),
    Action::new("audition", "", Effect::Write),
    Action::new("open", "", Effect::Write),
];

const A_MIDI: &[Action] = &[
    Action::new("ports", "the gear catalog and ports", Effect::Read),
    Action::new("monitor", "latest messages per port; pause / clear", Effect::Read),
    Action::new("send", "one channel-voice message", Effect::Live).probe(r#"{"device":"__no_such_port__","bytes":[144,60,100]}"#),
    Action::new("out", "a midiOut binding's args", Effect::Live).probe(r#"{"device":"__no_such_port__","args":{}}"#),
];

const A_BINDINGS: &[Action] = &[
    Action::new("devices", "devices, ports, presets", Effect::Read),
    Action::new("list", "a device's groups and overrides", Effect::Read),
    Action::new("map", "bound / total", Effect::Read),
    Action::new("preset", "switch preset", Effect::Live).probe(r#"{"preset":"__no_such_preset__"}"#),
    Action::new("preset_add", "copy the live preset", Effect::Live).probe(r#"{"device":"__no_such_device__"}"#),
    Action::new("preset_delete", "delete the live preset", Effect::Live).probe(r#"{"device":"__no_such_device__"}"#),
    Action::new("bind", "bind a control", Effect::Live).probe(r#"{"device":"__no_such_device__"}"#),
    Action::new("unbind", "unbind a control", Effect::Live).probe(r#"{"device":"__no_such_device__"}"#),
    Action::new("press", "press a control", Effect::Write),
    Action::new("targets", "the target registry", Effect::Read),
    Action::new("inject", "a gesture as MIDI through the router", Effect::Write),
    Action::new("jog", "a jog gesture", Effect::Write),
    Action::new("view", "a device's drawn layout", Effect::Read),
    Action::new("add_device", "add a device", Effect::Live).probe(r#"{"device":"__no_such_device__"}"#),
    Action::new("import", "import a map", Effect::Live).probe(r#"{"device":"__no_such_device__"}"#),
    Action::new("keys", "a device's key assignments", Effect::Read),
    Action::new("activity", "controls seen moving", Effect::Read),
];

const A_ROUTE: &[Action] = &[
    Action::new("status", "clips, routes, dests", Effect::Read),
    Action::new("set", "route a part", Effect::Write),
    Action::new("toggle", "toggle a route", Effect::Write),
    Action::new("clear", "clear routes", Effect::Write),
    Action::new("notes", "a clip's notes", Effect::Read),
    Action::new("offset", "dest offset ms", Effect::Write),
    Action::new("add_dest", "add a destination", Effect::Write),
    Action::new("open", "open the Route window", Effect::Write),
];

const A_LOG: &[Action] = &[
    Action::new("lines", "the log as words", Effect::Read),
    Action::new("arrangement", "the recording as an arrangement", Effect::Read),
    Action::new("save", "write it to a file", Effect::Live).probe(r#"{"from_bar":9999,"to_bar":9998}"#),
    Action::new("load", "read an arrangement", Effect::Write),
    Action::new("play", "replay a passage", Effect::Write),
    Action::new("stop", "stop replaying", Effect::Write),
    Action::new("clear", "clear the log", Effect::Live).probe(r#"{}"#),
    Action::new("open", "open the log window", Effect::Write),
];

const A_CLIPS: &[Action] = &[
    Action::new("list", "", Effect::Read),
    Action::new("scene_save", "", Effect::Write),
    Action::new("scene_launch", "", Effect::Write),
    Action::new("scene_rename", "", Effect::Write),
    Action::new("scene_delete", "", Effect::Write),
    Action::new("crop", "", Effect::Write),
    Action::new("sequence", "", Effect::Write),
    Action::new("retune", "", Effect::Write),
    Action::new("fx", "", Effect::Write),
    Action::new("fx_link", "", Effect::Write),
    Action::new("key_lock", "", Effect::Write),
    Action::new("row_key_lock", "", Effect::Write),
    Action::new("quantize", "", Effect::Write),
    Action::new("row_quantize", "", Effect::Write),
    Action::new("fade", "", Effect::Write),
    Action::new("row_fade", "", Effect::Write),
    Action::new("follow", "", Effect::Write),
    Action::new("row_follow", "", Effect::Write),
    Action::new("gain", "", Effect::Write),
    Action::new("rename", "", Effect::Write),
    Action::new("color", "", Effect::Write),
    Action::new("restore", "", Effect::Write),
    Action::new("forget", "", Effect::Write),
];

const A_WANDER: &[Action] = &[
    Action::new("status", "", Effect::Read),
    Action::new("on", "", Effect::Write),
    Action::new("off", "", Effect::Write),
    Action::new("pace", "", Effect::Write),
    Action::new("lock", "", Effect::Write),
    Action::new("grab", "", Effect::Write),
];

const A_AUTOMIX: &[Action] = &[
    Action::new("status", "", Effect::Read),
    Action::new("set", "", Effect::Write),
    Action::new("off", "", Effect::Write),
    Action::new("now", "", Effect::Write),
    Action::new("defer", "", Effect::Write),
];

const A_AUTONEXT: &[Action] = &[
    Action::new("status", "", Effect::Read),
    Action::new("on", "", Effect::Write),
    Action::new("off", "", Effect::Write),
    Action::new("toggle", "", Effect::Write),
    Action::new("set", "", Effect::Write),
    Action::new("up", "", Effect::Write),
    Action::new("down", "", Effect::Write),
];

const A_TRICK: &[Action] = &[
    Action::new("status", "every deck's tricks", Effect::Read),
    Action::new("on", "fire a trick", Effect::Write),
    Action::new("off", "release it", Effect::Write),
    Action::new("toggle", "toggle it", Effect::Write),
    Action::new("knob", "set the filter knob", Effect::Write),
];

const A_MOVES: &[Action] = &[
    Action::new("status", "what is armed", Effect::Read),
    Action::new("arm", "arm a move", Effect::Write),
    Action::new("cancel", "cancel a move", Effect::Write),
    Action::new("toggle", "arm / cancel", Effect::Write),
    Action::new("cancel_all", "cancel everything", Effect::Write),
];

const A_AUTODJ: &[Action] = &[
    Action::new("status", "on, bars, fade, style, effects, what it is doing, bars to the next handover", Effect::Read),
    Action::new("on", "turn Auto-DJ on (optionally with settings)", Effect::Write),
    Action::new("off", "turn it off; a transition in flight is let go", Effect::Write),
    Action::new("toggle", "on / off", Effect::Write),
    Action::new("set", "change bars, fade, style, effects (and `on`)", Effect::Write),
    Action::new("next", "run one transition now, on the next bar line", Effect::Write),
    Action::new("cancel", "let go of a transition in flight (stays on or off as it is)", Effect::Write),
];

const A_SETLIST: &[Action] = &[
    Action::new("list", "the saved sets (cid, name, segments, bars) and the working set's summary", Effect::Read),
    Action::new("get", "one set in full: a saved one by id, else the working set", Effect::Read)
        .with(&[Param::new("id", "string", "a saved set: its cid, a part of it, or its name")])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("new", "start an empty working set (or one from a hand-written `set`)", Effect::Write)
        .with(&[
            Param::new("name", "string", "its name (default New set)"),
            Param::new("set", "object", "a whole set's JSON (as `get` answers it): checked, its pairs against the loop sets"),
        ])
        .probe(r#"{"name":"probe"}"#),
    Action::new("generate", "make the working set from a recipe, the same each time for the same seed, from whole A / B pairs only", Effect::Write)
        .with(&[
            Param::new("segments", "integer", "how many (1-256, default 12)"),
            Param::new("genres", "string|array", "in order, the segments shared out among them: \"dnb then jungle\" or [\"dnb\",\"jungle\"] (default dnb)"),
            Param::new("bars", "integer", "bars each (4-128, default 16)"),
            Param::new("vary", "boolean", "a different transition each, never the same twice running (default true)"),
            Param::new("transition", "string", "the one transition when vary is false (default filter_exit)"),
            Param::new("inserts", "boolean", "Master filter sweeps and delay throws on the exits (default true)"),
            Param::new("seed", "integer", "the recipe's seed (default 1)"),
            Param::new("name", "string", "the set's name (default: its recipe)"),
            Param::new("save", "boolean", "save it too"),
        ])
        .probe(r#"{"segments":0}"#),
    Action::new("save", "save the working set under its content ID (an edited saved set replaces its old file)", Effect::Write)
        .with(&[Param::new("name", "string", "rename it first")])
        .probe(r#"{"name":"probe"}"#),
    Action::new("load", "a saved set into the working set", Effect::Write)
        .with(&[Param::new("id", "string", "its cid, a part of it, or its name").req()])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("delete", "delete a saved set", Effect::Write)
        .with(&[Param::new("id", "string", "its cid, a part of it, or its name").req()])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("duplicate", "save a copy of a saved set (or of the working one) under a new name; the copy is the working set", Effect::Write)
        .with(&[Param::new("id", "string", "the saved set; none: the working set"), Param::new("name", "string", "default: its name + copy")])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("edit", "change the working set (or a saved one, by id): insert, remove, move a segment, change its bars, transition, inserts, tag or source, rename; all or nothing", Effect::Write)
        .with(&[
            Param::new("op", "string", "insert | remove | move | bars | transition | inserts | tag | source | rename | note").req(),
            Param::new("id", "string", "a saved set to edit (it becomes the working set)"),
            Param::new("index", "integer", "the segment (1-based)"),
            Param::new("at", "integer", "insert: where it becomes (1-based; default the end)"),
            Param::new("to", "integer", "move: where it ends up (1-based)"),
            Param::new("source", "string|object", "insert / source: `dnb:82819#3` (a pair), `net:house:42`, `track:<cid>` or {kind, spec, group | cid}"),
            Param::new("segment", "object", "insert: a whole segment {source, bars, transition, inserts, tag}"),
            Param::new("bars", "integer", "bars (4-128)"),
            Param::new("transition", "string|object", "cut | bass_swap | filter_exit | echo_out | loop_roll | phrase_sync | vocoder_handoff, or {kind, bars}"),
            Param::new("transition_bars", "integer", "the move's bars (its sweep, tail or step)"),
            Param::new("inserts", "array", "[{kind: filter_sweep | delay_throw, bars, amount}]: Master automation over the last bars"),
            Param::new("tag", "object", "{energy: 1-5, genre}"),
            Param::new("name", "string", "rename"),
            Param::new("save", "boolean", "save it after"),
        ])
        .probe(r#"{"op":"__no_such_op__"}"#),
    Action::new("license", "the working set's license (or a saved one's, by id): no value reads it; a value sets it on the working set (a new CID; Save keeps it)", Effect::Write)
        .with(&[
            Param::new("value", "string", "CC0-1.0 (the default: public domain) | CC-BY-4.0 (reuse with credit) | all-rights-reserved"),
            Param::new("id", "string", "read a saved set's instead"),
            Param::new("author", "string", "name the author too (a signer identity, e.g. did:key:z…)"),
        ])
        .probe(r#"{"value":"__no_such_license__"}"#),
    Action::new("visibility", "the working set's visibility (or a saved one's, by id): no value reads it; public | private sets it (private: never shared or uploaded)", Effect::Write)
        .with(&[Param::new("value", "string", "public (the default) | private"), Param::new("id", "string", "read a saved set's instead")])
        .probe(r#"{"value":"__no_such_visibility__"}"#),
    Action::new("fork", "a remix of a saved set (or of the working one), saved and made the working set: it keeps the license, points at its parent (forkedFrom) and credits a CC BY parent's author; forking an all-rights-reserved set warns", Effect::Write)
        .with(&[
            Param::new("id", "string", "the saved set; none: the working set"),
            Param::new("name", "string", "default: its name + (remix)"),
            Param::new("author", "string", "who made the fork (default $DJ_AUTHOR)"),
        ])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("share", "a set's CID and canonical object, for sharing by CID; refused when the set is private", Effect::Read)
        .with(&[Param::new("id", "string", "the saved set; none: the working set")])
        .probe(r#"{"id":"__no_such_set__"}"#),
];

const A_TRACK: &[Action] = &[
    Action::new("list", "the saved tracks: cid, music_cid, name, license, visibility, author, forkedFrom, credits", Effect::Read),
    Action::new("get", "one saved track in full (its object, CIDs, rights and credit line); with deck: the saved track that deck plays", Effect::Read)
        .with(&[Param::new("id", "string", "its cid, a part of it, or its name"), Param::new("deck", "string", "A or B")])
        .probe(r#"{"id":"__no_such_track__"}"#),
    Action::new("save", "save what a deck plays as a track (default CC0-1.0 and public)", Effect::Write)
        .with(&[
            Param::new("deck", "string", "A or B").req(),
            Param::new("name", "string", "default: the deck's title"),
            Param::new("license", "string", "CC0-1.0 (default) | CC-BY-4.0 | all-rights-reserved"),
            Param::new("visibility", "string", "public (default) | private"),
            Param::new("author", "string", "a signer identity (default $DJ_AUTHOR)"),
        ])
        .probe(r#"{"deck":"Z"}"#),
    Action::new("license", "a saved track's license: no value reads it; a value sets it (a new object, a new CID; the old file goes)", Effect::Write)
        .with(&[
            Param::new("id", "string", "its cid, a part of it, or its name").req(),
            Param::new("value", "string", "CC0-1.0 | CC-BY-4.0 | all-rights-reserved"),
        ])
        .probe(r#"{"id":"__no_such_track__"}"#),
    Action::new("visibility", "a saved track's visibility: no value reads it; public | private sets it (a new CID; private is never shared or uploaded)", Effect::Write)
        .with(&[Param::new("id", "string", "its cid, a part of it, or its name").req(), Param::new("value", "string", "public | private")])
        .probe(r#"{"id":"__no_such_track__"}"#),
    Action::new("fork", "save a remix of a saved track: it keeps the license, points at its parent (forkedFrom) and carries every credit, adding the parent's author when it is CC BY; forking an all-rights-reserved track warns", Effect::Write)
        .with(&[
            Param::new("id", "string", "the parent: its cid, a part of it, or its name").req(),
            Param::new("name", "string", "default: its name + (remix)"),
            Param::new("author", "string", "who made the fork (default $DJ_AUTHOR)"),
        ])
        .probe(r#"{"id":"__no_such_track__"}"#),
    Action::new("share", "a saved track's CID, music CID and canonical object, for sharing by CID; refused when it is private", Effect::Read)
        .with(&[Param::new("id", "string", "its cid, a part of it, or its name").req()])
        .probe(r#"{"id":"__no_such_track__"}"#),
    Action::new("load", "play a saved track on a deck (composed on a worker, from its top)", Effect::Write)
        .with(&[Param::new("id", "string", "its cid, a part of it, or its name").req(), Param::new("deck", "string", "A or B").req()])
        .probe(r#"{"id":"__no_such_track__","deck":"A"}"#),
    Action::new("delete", "delete a saved track", Effect::Write)
        .with(&[Param::new("id", "string", "its cid, a part of it, or its name").req()])
        .probe(r#"{"id":"__no_such_track__"}"#),
];

const A_LIVE: &[Action] = &[
    Action::new("eval", "parse, check, compile and stage a btw program: it lands on the next bar (an error changes nothing: the last good program plays on)", Effect::Live)
        .with(&[
            Param::new("text", "string", "the program (docs/BTW.md); none: what the btw window holds"),
            Param::new("name", "string", "a name for it (kept if it is saved later)"),
        ])
        .probe(r#"{"text":"deck A: kick"}"#),
    Action::new("check", "the same without staging: diagnostics (line, column, expected, found), the net CIDs and what would change", Effect::Read)
        .with(&[Param::new("text", "string", "the program; none: the window's text")])
        .probe(r#"{"text":"deck A: kick"}"#),
    Action::new("get", "the running program (canonical text, CID, nets), the section and bar, the staged one, the last errors, warnings (a muted strip), what happened lately", Effect::Read),
    Action::new("save", "keep the running program (or `text`) as a licensed .btw object under its content ID (CC0 and public unless said)", Effect::Write)
        .with(&[
            Param::new("name", "string", "its name"),
            Param::new("text", "string", "a program to save instead of the running one"),
            Param::new("license", "string", "CC0-1.0 (default), CC-BY-4.0, all-rights-reserved"),
            Param::new("visibility", "string", "public (default) or private"),
            Param::new("author", "string", "who made it (default $DJ_AUTHOR)"),
            Param::new("genre", "string", "a genre of the bitwrap ontology (`techno`, `genre:techno`, `bw:Techno` or its IRI; \"\" clears it): a new object of the same program; the text's `#! genre:` header line says the same"),
            Param::new("tags", "string|array", "terms of the ontology, `acid, four on floor, drop` or a list (local name, label, `kind:name` or IRI; [] clears them); the text's `#! tags:` header line says the same"),
        ])
        .probe(r#"{"text":"deck A: kick"}"#),
    Action::new("load", "a saved program (by CID, part of one, or name) or a .btw file (`path`) into the window, and (eval, default true) staged to play on the next bar", Effect::Live)
        .with(&[
            Param::new("id", "string", "its cid, a part of it, or its name"),
            Param::new("path", "string", "a .btw file on disk instead (it is kept too)"),
            Param::new("eval", "boolean", "stage it too (default true)"),
        ])
        .probe(r#"{"id":"__no_such_snippet__"}"#),
    Action::new("list", "the saved programs (cid, object, name, lines, license, visibility, author, forkedFrom)", Effect::Read),
    Action::new("stop", "end the program: the decks play on, its control lines and ramps end (what a line still holds changed, a muted strip or a filter, is put back)", Effect::Write),
    Action::new("fmt", "the canonical form of a program: its text as the CID sees it, the CID, and the comments (which are not part of it)", Effect::Read)
        .with(&[Param::new("text", "string", "the program; none: the window's text")])
        .probe(r#"{"text":"deck A: kick"}"#),
    Action::new("share", "a self-sealing link, https://beats.bitwrap.io/p#<CID>.<deflated canonical text>[.<sig>][~<notes>]; a private program is refused", Effect::Read)
        .with(&[
            Param::new("id", "string", "a saved program (cid, part of one, name); none: the running program, else `text`"),
            Param::new("text", "string", "a program to share"),
            Param::new("sig", "string", "the author's signature over the CID, to carry (it is the wallet's to make and check)"),
            Param::new("notes", "boolean", "carry the comments, unsealed (default false)"),
        ])
        .probe(r#"{"id":"__no_such_snippet__"}"#),
    Action::new("open", "open a link or a CID: decode, canonicalise, compile, recompute the CID and refuse a link whose CID is not its text's; into the window, staged unless eval is false", Effect::Live)
        .with(&[
            Param::new("url", "string", "a btw link, its fragment, `CID.text`, or a bare CID (found in storage)").req(),
            Param::new("eval", "boolean", "stage it too (default false)"),
        ])
        .probe(r#"{"url":"__not_a_link__"}"#),
    Action::new("fork", "a fork of a saved program: the same program, forkedFrom its CID, its license and credits carried (a CC BY parent is credited), saved", Effect::Write)
        .with(&[
            Param::new("id", "string", "the parent: cid, part of one, or name").req(),
            Param::new("name", "string", "default \"<name> (fork)\""),
            Param::new("author", "string", "who forks it (default $DJ_AUTHOR)"),
        ])
        .probe(r#"{"id":"__no_such_snippet__"}"#),
    Action::new("license", "a saved program's license (value: CC0-1.0, CC-BY-4.0, all-rights-reserved); none: read it", Effect::Write)
        .with(&[Param::new("id", "string", "which").req(), Param::new("value", "string", "the license")])
        .probe(r#"{"id":"__no_such_snippet__"}"#),
    Action::new("visibility", "a saved program's visibility (value: public or private; a private program is never shared); none: read it", Effect::Write)
        .with(&[Param::new("id", "string", "which").req(), Param::new("value", "string", "public or private")])
        .probe(r#"{"id":"__no_such_snippet__"}"#),
    Action::new("export", "write a saved program as a .btw file (a program with #! header lines for its name and rights) to `path`", Effect::Live)
        .with(&[Param::new("id", "string", "which").req(), Param::new("path", "string", "a file or a folder").req()])
        .probe(r#"{"id":"__no_such_snippet__","path":"/nonexistent"}"#),
    Action::new("import", "take a .btw file (`path`) or its text (`text`) in as a saved program, with the rights its header says", Effect::Write)
        .with(&[Param::new("path", "string", "a .btw file"), Param::new("text", "string", "or the file's text"), Param::new("name", "string", "a name if the file has none")])
        .probe(r#"{"path":"/nonexistent/none.btw"}"#),
    Action::new("generation", "the CID of generation N of an `evolve`, made from generation 0 and the rule alone (no chain kept: generation 1,000,000 costs the length of the orbit); also the evolve's tail and period", Effect::Read)
        .with(&[
            Param::new("gen", "integer", "the generation (0 is the program as written)").req(),
            Param::new("scope", "string", "the section the evolve is in (default the first evolve)"),
            Param::new("text", "string", "the program; none: the running one, else the window's"),
        ])
        .probe(r#"{"gen":1,"text":"deck A: kick"}"#),
    Action::new("instruments", "every voice, family word, drum kit and sound palette the engine has: what `inst=`, `kit=` and `sound=` accept; an unknown name in a program is an error with suggestions", Effect::Read),
    Action::new("jsonld", "a program as linked data (JSON-LD): its CID as @id, type djrust:Program, its genre and tags (the `#! genre:` / `#! tags:` header lines: IRIs of the bitwrap ontology), key, tempo, sections, license, forkedFrom and credits, and its Euclidean rhythms linked to the ontology's Euclidean rhythm class; the document a saved program keeps beside it as `<object cid>.jsonld`", Effect::Read)
        .with(&[
            Param::new("id", "string", "a saved program (cid, part of one, name); none: `text`, else the window's text"),
            Param::new("text", "string", "a program, or a .btw file's text: its `#!` header lines give its genre and tags, and its rights when it has license / visibility / author lines"),
        ])
        .probe(r#"{"id":"__no_such_snippet__"}"#),
    Action::new("terms", "the vocabulary the `#! genre:` and `#! tags:` header lines are checked against (a cached copy of the bitwrap ontology, https://ontology.bitwrap.io/ns#): each term's kind, local name, IRI and label, and where the copy is from", Effect::Read)
        .with(&[Param::new("kind", "string", "genre, instrument, scale, groove, section, family, role, rhythm, progression, structure or concept; none: all of them")])
        .probe(r#"{"kind":"__no_such_kind__"}"#),
    Action::new("test", "judge a program's `assert` statements (F12): the static ones (`ends`, `period`, `length`, `key`) are re-checked, the `level` ones are measured on an offline render (each deck's RMS in the section, from the renderer's per-strip stems) and reported pass or fail with the measured values; `ok` is false when one fails, and a program with an error is refused; refused while a program is running or staged (the render would hold the window and its bar clock): `stop` first, or `djctl btw test FILE`", Effect::Read)
        .with(&[
            Param::new("text", "string", "the program; none: the running one, else the window's"),
            Param::new("id", "string", "a saved program (cid, part of one, name) instead"),
            Param::new("sample_rate", "number", "the render's sample rate (default 44100)"),
        ])
        .probe(r#"{"text":"deck A: kick"}"#),
    Action::new("perform", "the performed inputs of the running program (`knob(3)`, `pad(5)`, `fader(A)`, `held(key)` in its control values and `when`s): what it reads and the performance it has logged, every value read by program bar. `do`: status (default), start (a new take), stop (stop logging; the values are still read), save (keep the take as a performance object under its own CID), list (the saved performances)", Effect::Write)
        .with(&[Param::new("do", "string", "status (default), start, stop, save or list")]),
    Action::new("replay", "play a saved performance back: its program is evaluated afresh and its inputs read the performance's values, by bar, instead of the controllers, so its controls are the same calls with the same numbers; a performance whose program is not saved here, or is not the one the text names, is refused", Effect::Live)
        .with(&[Param::new("id", "string", "the performance's CID (or its head or tail)").req()])
        .probe(r#"{"id":"__no_such_performance__"}"#),
];

const A_QUEUE: &[Action] = &[
    Action::new("status", "state (stopped / starting / playing / paused), now and next, bars left, the queued sets, the last word", Effect::Read),
    Action::new("add", "queue a saved set (by id) or the working one; with `segments` it generates one first", Effect::Write)
        .with(&[
            Param::new("id", "string", "a saved set; none: the working set"),
            Param::new("times", "integer", "queue it this many times (1-16)"),
            Param::new("segments", "integer", "generate first: the recipe's params (genres, bars, vary, inserts, seed, name)"),
        ])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("clear", "empty the queue (the decks play on; what the queue had armed is let go)", Effect::Write),
    Action::new("play", "play: from the top (or `from`) when stopped, on from where it was when paused", Effect::Live)
        .with(&[Param::new("from", "integer", "the segment to start at (1-based across the queue)")])
        .probe(r#"{"from":999}"#),
    Action::new("pause", "pause both decks; the count stops with them", Effect::Live).probe("{}"),
    Action::new("stop", "an echo out on the next bar, then the decks stop (at once when paused)", Effect::Live).probe("{}"),
    Action::new("next", "skip to the next segment: its transition runs now (at the end it ends the set)", Effect::Live).probe("{}"),
    Action::new("jump", "go to a segment: its transition runs now", Effect::Live)
        .with(&[
            Param::new("n", "integer", "the segment (1-based across the queue)"),
            Param::new("set", "integer", "or: the set in the queue (1-based) ..."),
            Param::new("segment", "integer", "... and the segment in it"),
        ])
        .probe(r#"{"n":999}"#),
    Action::new("loop", "loop the queue (after the last segment, the first); on, or flip", Effect::Write)
        .with(&[Param::new("on", "boolean", "default: flip")])
        .probe(r#"{"on":false}"#),
];

const A_FREEZE: &[Action] = &[
    Action::new("status", "read", Effect::Read),
    Action::new("set", "knobs", Effect::Write),
    Action::new("on", "freeze", Effect::Write),
    Action::new("off", "thaw", Effect::Write),
    Action::new("toggle", "toggle", Effect::Write),
    Action::new("scrub", "scrub the frozen audio", Effect::Write),
    Action::new("phrase", "freeze N bars ending on the next bar line of the input's grid", Effect::Write)
        .with(&[Param::new("bars", "integer", "1..8 (default 1; at most 16 beats)")]),
];

const A_VOCODER: &[Action] = &[
    Action::new("status", "the vocoder", Effect::Read),
    Action::new("set", "set sources, knobs, switches", Effect::Write),
    Action::new("cycle_modulator", "next modulator", Effect::Write),
    Action::new("cycle_carrier", "next carrier", Effect::Write),
    Action::new("swap", "swap modulator and carrier", Effect::Write),
    Action::new("preset_save", "save the synth preset", Effect::Live).probe(r#"{}"#),
    Action::new("preset_delete", "delete one", Effect::Live).probe(r#"{"name":"__no_such_preset__"}"#),
    Action::new("preset_next", "next preset", Effect::Write),
    Action::new("preset_prev", "previous preset", Effect::Write),
    Action::new("list", "every instance", Effect::Read),
    Action::new("add", "add an instance", Effect::Write),
    Action::new("remove", "remove an instance", Effect::Write),
    Action::new("select", "select an instance", Effect::Write),
];

const A_POST: &[Action] = &[
    Action::new("status", "the live chain", Effect::Read),
    Action::new("set", "set on / preset / chain / step", Effect::Live).probe(r#"{"preset":"__no_such_preset__"}"#),
    Action::new("process", "process a file", Effect::Write),
    Action::new("measure", "measure a file", Effect::Read),
    Action::new("presets", "list presets", Effect::Read),
];

const A_HEOS: &[Action] = &[
    Action::new("status", "", Effect::Read),
    Action::new("volume", "", Effect::Live).probe(r#"{"zone":"__none__","level":1}"#),
    Action::new("mute", "", Effect::Live).probe(r#"{"zone":"__none__"}"#),
    Action::new("play", "", Effect::Live).probe(r#"{"zone":"__none__"}"#),
    Action::new("pause", "", Effect::Live).probe(r#"{"zone":"__none__"}"#),
    Action::new("stop", "", Effect::Live).probe(r#"{"zone":"__none__"}"#),
    Action::new("group", "", Effect::Live).probe(r#"{"zones":["__none__"]}"#),
    Action::new("ungroup", "", Effect::Live).probe(r#"{"zone":"__none__"}"#),
    Action::new("stream", "", Effect::Live).probe(r#"{"zones":["__none__"]}"#),
    Action::new("stream_stop", "", Effect::Live).probe(r#"{"zones":["__none__"]}"#),
    Action::new("stream_server_stop", "", Effect::Write),
    Action::new("select", "", Effect::Live).probe(r#"{"zones":["__none__"]}"#),
    Action::new("discover", "", Effect::Live).probe("!"),
    Action::new("connect", "", Effect::Live).probe(r#"{}"#),
    Action::new("refresh", "", Effect::Live).probe("!"),
    Action::new("open", "", Effect::Write),
];

const A_CONFIG: &[Action] = &[
    Action::new("get", "read a key or section", Effect::Read),
    Action::new("set", "set keys", Effect::Live).probe(r#"{"key":"__no.such.key__","value":1}"#),
    Action::new("diff", "against defaults / file / launch", Effect::Read),
    Action::new("save", "write the file", Effect::Live).probe(r#"{}"#),
    Action::new("reload", "re-read the file", Effect::Write),
    Action::new("keys", "the key table", Effect::Read),
    Action::new("path", "the file's path", Effect::Read),
];

const A_PERF: &[Action] = &[
    Action::new("status", "", Effect::Read),
    Action::new("on", "", Effect::Write),
    Action::new("off", "", Effect::Write),
    Action::new("toggle", "", Effect::Write),
    Action::new("reset", "", Effect::Write),
    Action::new("set", "", Effect::Write),
];

const A_BROWSE: &[Action] = &[
    Action::new("status", "", Effect::Read),
    Action::new("sources", "", Effect::Read),
    Action::new("packs", "", Effect::Read),
    Action::new("list", "", Effect::Read),
    Action::new("search", "", Effect::Read),
    Action::new("load", "", Effect::Write),
    Action::new("audition", "", Effect::Write),
    Action::new("stop", "", Effect::Write),
    Action::new("gain", "", Effect::Write),
    Action::new("open", "", Effect::Write),
    Action::new("rescan", "", Effect::Write),
    Action::new("select", "", Effect::Write),
];

const P_SPEC: Param = Param::new("spec", "string", "a collection: acid-bass, a genre (techno, dnb), or genre:seed (dnb:2602)");
const P_DECK_AB: Param = Param::new("deck", "string", "A or B");

const A_LOOPSETS: &[Action] = &[
    Action::new("status", "the Loops window: the collection picked, its key and tempo, what each deck holds, the lit pair", Effect::Read),
    Action::new("collections", "the picker: the Acid Bass bank, every genre's curated set, the kept (starred) seeded sets", Effect::Read),
    Action::new("groups", "the picked collection's groups: each group's A and B loop with name, description, tempo and key", Effect::Read)
        .with(&[Param::new("spec", "string", "a collection to read instead of the picked one (built if needed)")]),
    Action::new("pick", "show a collection (the picker, or a typed genre:seed); the set is built before the answer", Effect::Write)
        .with(&[P_SPEC.req()])
        .probe(r#"{"spec":"techno"}"#),
    Action::new("new_set", "the New set button: a fresh seeded set of a genre (the picked one's by default), shown at once", Effect::Write)
        .with(&[Param::new("genre", "string", "default the picked collection's genre"), Param::new("seed", "integer", "1..100000; default a new random one")]),
    Action::new("keep", "the star: keep a seeded set in the picker (on=false lets it go; omitted toggles)", Effect::Write)
        .with(&[P_SPEC, Param::new("on", "boolean", "true keeps, false lets go, omitted toggles")]),
    Action::new("drums", "the Drums checkbox: loops with their drums, or the line alone", Effect::Write)
        .with(&[Param::new("on", "boolean", "omitted toggles")]),
    Action::new("load", "a pad: one loop onto a deck and play it, bar-locked to the other deck when that one plays", Effect::Write)
        .with(&[
            P_DECK_AB.req(),
            Param::new("id", "string", "the loop's id in its collection: 3A, 3B"),
            Param::new("group", "integer", "with side: the group number"),
            Param::new("side", "string", "A or B: which of the group's two loops (default the deck's own letter)"),
            P_SPEC,
        ])
        .probe(r#"{"deck":"A","id":"1A"}"#),
    Action::new("load_pair", "the A + B pad: group N's A on deck A and its B on deck B, both playing, locked on the bar", Effect::Write)
        .with(&[Param::new("group", "integer", "the group number, 1..").req(), P_SPEC])
        .probe(r#"{"group":1}"#),
    Action::new("open", "show or hide the Loops window", Effect::Write)
        .with(&[Param::new("on", "boolean", "omitted toggles")]),
];


const P_TARGET: Param = Param::new("target", "string", "A, B, Wheel or Master (default the window's own aim: the lead deck until picked)");

const A_WINDOWS: &[Action] = &[
    Action::new("list", "every window and whether it is open", Effect::Read),
    Action::new("open", "open a window (the top bar's button)", Effect::Write)
        .with(&[Param::new("window", "string", "td3 mpk kits bindings log wheel loops perform effects inserts aux net heos menu map midi route vocoder browser perf set").req()])
        .probe(r#"{"window":"__no_such_window__"}"#),
    Action::new("close", "close a window", Effect::Write)
        .with(&[Param::new("window", "string", "as open").req()])
        .probe(r#"{"window":"__no_such_window__"}"#),
    Action::new("toggle", "flip a window (the button), or set it with on", Effect::Write)
        .with(&[Param::new("window", "string", "as open").req(), Param::new("on", "boolean", "")])
        .probe(r#"{"window":"__no_such_window__"}"#),
    Action::new("fullscreen", "make a window fill the screen (only `live`, the btw editor, so far; `on` false returns it to its window): large monospace text, the stage theme, the section / bar / phrase header, and the firing markers; Esc returns", Effect::Write)
        .with(&[Param::new("window", "string", "live (or btw): the btw editor").req(), Param::new("on", "boolean", "default true")])
        .probe(r#"{"window":"__no_such_window__"}"#),
    Action::new("follow", "the btw editor follows the song (`on` true) or stays where it is (false; no `on` flips it): it scrolls to each section as it starts and to the lines firing in a long one, edge arrows mark ramps and timed lines that fire off-screen; a manual scroll pauses it for 8 s or until the next section; key L", Effect::Write)
        .with(&[Param::new("window", "string", "live (or btw): the btw editor").req(), Param::new("on", "boolean", "default: flip it")])
        .probe(r#"{"window":"__no_such_window__"}"#),
    Action::new("closeall", "close every window", Effect::Write),
];

const A_INSERTS: &[Action] = &[
    Action::new("status", "every target's chain: effects, switches, knobs with their display; or one target's", Effect::Read)
        .with(&[P_TARGET]),
    Action::new("on", "switch an insert effect (filter, crush, delay, reverb) on or off", Effect::Write)
        .with(&[P_TARGET, Param::new("effect", "string", "filter | crush | delay | reverb").req(), Param::new("on", "boolean", "default true")])
        .probe(r#"{"effect":"__no_such_effect__"}"#),
    Action::new("set", "turn a knob (filter.*, crush.bits, delay.time, delay.mix, ...) to 0..1", Effect::Write)
        .with(&[P_TARGET, Param::new("param", "string", "a knob id: see status").req(), Param::new("value", "number", "0..1").req()])
        .probe(r#"{"param":"__no_such_knob__","value":0.5}"#),
    Action::new("off", "ALL OFF: every insert on every target off; the knobs stay", Effect::Write),
];

const A_EFFECTS: &[Action] = &[
    Action::new("status", "the window's aim and length, and what is on or waiting for its bar", Effect::Read).with(&[P_TARGET]),
    Action::new("catalogue", "every pad: id, label, category, family, whether it needs a net deck or a deck", Effect::Read),
    Action::new("tap", "a pad clicked: off on the next bar if it is on, else on for the target's length", Effect::Write)
        .with(&[P_TARGET, Param::new("id", "string", "a pad id from catalogue").req()])
        .probe(r#"{"id":"__no_such_pad__"}"#),
    Action::new("hold", "a transition pad pressed (held until release)", Effect::Write)
        .with(&[P_TARGET, Param::new("id", "string", "a pad id").req()])
        .probe(r#"{"id":"__no_such_pad__"}"#),
    Action::new("release", "a held pad let go", Effect::Write)
        .with(&[P_TARGET, Param::new("id", "string", "a pad id").req()])
        .probe(r#"{"id":"__no_such_pad__"}"#),
    Action::new("length", "the target's engagement: 1-4 bars or latch", Effect::Write)
        .with(&[P_TARGET, Param::new("length", "integer|string", "1-4 or latch").req()]),
    Action::new("sweep", "the sweep categories' own bars (1-4)", Effect::Write)
        .with(&[P_TARGET, Param::new("bars", "integer", "1-4").req()]),
    Action::new("knob", "the filter knob: 0 low-pass, 0.5 open, 1 high-pass (answers at once)", Effect::Write)
        .with(&[P_TARGET, Param::new("value", "number", "0..1").req()]),
    Action::new("panic", "end everything on the target now", Effect::Write).with(&[P_TARGET]),
    Action::new("target", "aim the window at A, B, Wheel or Master", Effect::Write).with(&[P_TARGET.req()]),
];


const A_WHEEL: &[Action] = &[
    Action::new("status", "the instrument (law, path, notes, key, motor, speed, level, trigger), play state, meters", Effect::Read),
    Action::new("presets", "the instruments to load", Effect::Read),
    Action::new("play", "Play (the first play kicks the motor on)", Effect::Write),
    Action::new("pause", "Pause: everything held where it is, silent", Effect::Write),
    Action::new("toggle", "the Play / Pause button", Effect::Write),
    Action::new("load", "an instrument (wound by a worker: poll status until busy is false)", Effect::Write)
        .with(&[Param::new("preset", "string", "an id from presets").req()])
        .probe(r#"{"preset":"__no_such_instrument__"}"#),
    Action::new("motor", "the motor of a cord instrument: on / off and its speed", Effect::Write)
        .with(&[Param::new("on", "boolean", ""), Param::new("speed", "number", "m/s, 0..25 (the instruments are wound for 0.8-4.5)")]),
    Action::new("interval", "a gravity instrument drops by itself every N seconds (0 off), or every N beats of a deck's tempo so it sits on the grid", Effect::Write)
        .with(&[
            Param::new("seconds", "number", "0, or 0.1..1 in hundredths"),
            Param::new("beats", "string|number", "a note length in beats, \"1/2\" or 0.5 or 1/4: seconds = beats x 60 / the deck's BPM (the followed deck unless `deck`); the answer says what it came to and how far off the grid hundredths leave it"),
            Param::new("deck", "string", "A or B: whose tempo `beats` is of (default the one the wheel follows)"),
        ]),
    Action::new("sync", "the GUI's Sync: set the motor so the wheel's tempo settles on a whole-number multiple or divisor of a deck's: the nearest, or `ratio` (\"1/4\" is a quarter of the deck's tempo, \"2\" twice it)", Effect::Write)
        .with(&[
            Param::new("deck", "string", "A or B (default the one the wheel follows)"),
            Param::new("ratio", "string|number", "a whole multiple (2, 3) or a divisor (1/2, 1/4); none: the nearest rung to now"),
        ])
        .probe(r#"{"deck":"Z"}"#),
    Action::new("step", "the GUI's rung buttons: one rung of the tempo ladder up or down against a deck, within the motor's range", Effect::Write)
        .with(&[
            Param::new("deck", "string", "A or B (default the one the wheel follows)"),
            Param::new("up", "boolean", "default true; false steps down (and on into 1/2, 1/3 ...)"),
        ])
        .probe(r#"{"deck":"Z"}"#),
    Action::new("follow", "which deck's tempo the wheel's sync rung and `interval beats` read (the GUI's Deck A / Deck B buttons)", Effect::Write)
        .with(&[Param::new("deck", "string", "A or B").req()])
        .probe(r#"{"deck":"Z"}"#),
    Action::new("pluck", "a drop / swing / pluck now", Effect::Write)
        .with(&[Param::new("tube", "integer", "a tube's pad (0..) on a gravity instrument; none: the whole instrument")]),
    Action::new("level", "the wheel's level, dB -20..40", Effect::Write)
        .with(&[Param::new("db", "number", "").req()]),
    Action::new("trigger", "the trigger: a load across the coil (on=false: the fixed load)", Effect::Write)
        .with(&[Param::new("on", "boolean", ""), Param::new("pull", "number", "0 open .. 0.5 matched .. 1 short; default 0.5")]),
    Action::new("key", "the key every note is re-wound for", Effect::Write)
        .with(&[Param::new("root", "string", "C, F#3, Bb2 ...: without an octave it keeps the current one"), Param::new("transpose", "integer", "-24..24")]),
    Action::new("brightness", "the coil and magnet sizes, 0..1", Effect::Write)
        .with(&[Param::new("value", "number", "0..1").req()]),
    Action::new("sequence", "the notes the path plays, in order (\"C E G B C B G E\"); none: the preset's own", Effect::Write)
        .with(&[Param::new("notes", "string", "")]),
];

const A_MENU: &[Action] = &[
    Action::new("status", "keep awake (and whether it is holding), the Toy Box pack, version, features", Effect::Read),
    Action::new("keep_awake", "keep the computer awake while something plays (on; omitted flips it)", Effect::Write),
    Action::new("toybox", "install or update the Toy Box SoundFont (sf2 builds): writes a file, used on the next launch", Effect::Live)
        .probe("!"),
];

const A_CONTROLLERS: &[Action] = &[
    Action::new("status", "the Mixxx mapping folders and how many mappings are in them, every input port and what drives it (a profile, a Mixxx mapping, nothing and why)", Effect::Read),
    Action::new("list", "the Mixxx mappings found (file, name, controller id), optionally narrowed by a word", Effect::Read)
        .with(&[Param::new("filter", "string", "part of a mapping's name or file")]),
    Action::new("load", "put a Mixxx mapping on an input port now (a hand-made profile's port is refused); answers what mapped and what did not", Effect::Write)
        .with(&[
            Param::new("port", "string", "the input port, as `status` lists it").req(),
            Param::new("mapping", "string", "a file name or part of a mapping's name, from `list`").req(),
        ])
        .probe(r#"{"port":"__no_such_port__","mapping":"__no_such_mapping__"}"#),
    Action::new("unload", "take a Mixxx mapping off a port (by port or device name); the controller stops driving the decks", Effect::Write)
        .with(&[
            Param::new("port", "string", "the port it was loaded on"),
            Param::new("device", "string", "or the device name, `Mixxx: …`"),
        ])
        .probe(r#"{"port":"__no_such_port__"}"#),
    Action::new("report", "what a loaded mapping bound and what it left, control by control, with the reason", Effect::Read)
        .with(&[
            Param::new("device", "string", "the device name, `Mixxx: …`"),
            Param::new("port", "string", "or its port"),
        ])
        .probe(r#"{"device":"__no_such_device__"}"#),
    Action::new("rescan", "read the Mixxx mapping folders again (after installing Mixxx or adding a mapping)", Effect::Write),
    Action::new("map", "remember that a port gets a mapping (or `none`): the config's mixxx.map override, applied now", Effect::Write)
        .with(&[
            Param::new("port", "string", "part of the port's name").req(),
            Param::new("mapping", "string", "a file name or mapping name, or none").req(),
        ])
        .probe(r#"{"port":"","mapping":""}"#),
];

const A_ANALYSIS: &[Action] = &[
    Action::new("status", "the analysis worker, Beat This! and its models, the settings, the grids found", Effect::Read),
    Action::new(
        "beats",
        "find the beats, tempo and bar 1 of a capture, a loop, an input's history, a file or the library (off the audio thread; cached by content hash)",
        Effect::Live,
    )
    .with(&[
        Param::new("target", "string", "capture | clip | input | file | library | vox").req(),
        Param::new("id", "string", "the clip's or library loop's id (a clip, a capture, a library loop)"),
        Param::new("input", "string", "the input's label (target input)"),
        Param::new("path", "string", "an audio file (target file) or a folder (target vox)"),
        Param::new("seconds", "number", "how much of the input's recording (default analysis.history_seconds)"),
        Param::new("force", "boolean", "analyse again even if the content is cached"),
        Param::new("trim", "boolean", "target capture: also trim it to start on the downbeat and run whole bars (new captures are trimmed by themselves)"),
    ])
    .probe(r#"{"target":"clip","id":"__no_such_clip__"}"#),
    Action::new("grid", "the grid already found for a target (no analysis started)", Effect::Read).with(&[
        Param::new("target", "string", "capture | clip | input | file | library").req(),
        Param::new("id", "string", ""),
        Param::new("input", "string", ""),
        Param::new("path", "string", ""),
    ]),
    Action::new("set", "neural, capture_trim, min_confidence, snap, history_auto, history_seconds", Effect::Write).with(&[
        Param::new("neural", "boolean", "use Beat This! (models fetched on first use); off: the DSP detector only"),
        Param::new("capture_trim", "boolean", "trim new captures to start on the downbeat and run whole bars, and give them the detected tempo"),
        Param::new("min_confidence", "number", "a grid below this (0..1) is shown but never used to trim"),
        Param::new("snap", "string", "off | beat | bar: a released history scrub lands on this line of the input's grid"),
        Param::new("history_auto", "boolean", "analyse an input's history by itself while it has signal"),
        Param::new("history_seconds", "number", "how many seconds of an input's recording an analysis listens to"),
    ]),
    Action::new("models", "the model files: where, pinned size and SHA-256, whether they are there", Effect::Read),
    Action::new("fetch_models", "download the Beat This! models now (about 11 MB, checksum-verified)", Effect::Live).probe("!"),
    Action::new("retry", "try loading Beat This! again after a failure", Effect::Live).probe("!"),
];

const A_INPUT: &[Action] = &[
    Action::new("status", "the strip (what a call with no knob and no verb answers)", Effect::Read),
    Action::new(
        "calibrate_latency",
        "measure how late the input is against the master clock (a source slaved to djapp's MIDI clock, e.g. the TD-3) and suggest or set latency_ms",
        Effect::Live,
    )
    .with(&[
        Param::new("seconds", "number", "how much of the recording to listen to (default 12, 4..30)"),
        Param::new("apply", "boolean", "set latency_ms from the answer when it is confident (default false: only suggest)"),
        Param::new("min_confidence", "number", "the confidence apply needs (default 0.7)"),
    ])
    .probe(r#"{"input":"__no_such_strip__"}"#),
    Action::new("latency", "the last calibration of each input: ms, confidence, method, when", Effect::Read),
];

const A_NET: &[Action] = &[
    Action::new("status", "the host, port, pairing code, trust, discovery, peers with their clocks, the log", Effect::Read),
    Action::new("track", "a beats track by CID, checked and composed here", Effect::Write)
        .with(&[Param::new("deck", "string", "A or B"), Param::new("cid", "string", ""), Param::new("payload", "object", ""), Param::new("play", "boolean", ""), Param::new("sync", "boolean", "")]),
    Action::new("glance", "the decks, mixer and stream, compact", Effect::Read),
    Action::new("stream", "the phone's master stream into the Phone strip", Effect::Live)
        .with(&[Param::new("url", "string", "http://PHONE:PORT/master.djl").req()])
        .probe("{}"),
    Action::new("stream_stop", "stop the phone stream", Effect::Write),
    Action::new("stream_status", "the stream: state, tempo, phase, latency, drift, drop-outs", Effect::Read),
    Action::new("aux_sync", "sync the aux stream", Effect::Write),
    Action::new("aux_lock", "lock the aux stream's phase", Effect::Write).with(&[Param::new("on", "boolean", "")]),
    Action::new("aux_nudge", "nudge the aux stream", Effect::Write).with(&[Param::new("beats", "number", "")]),
    Action::new("aux_trim", "trim the aux stream", Effect::Write).with(&[Param::new("ms", "number", "±50")]),
    Action::new("aux_latency", "the aux stream's latency", Effect::Write).with(&[Param::new("ms", "number", "30-1000")]),
    Action::new("aux_match", "put the master deck on the stream's tempo", Effect::Write),
    Action::new("code", "a new pairing code", Effect::Write),
    Action::new("trust", "who may pair: code, open or off", Effect::Write).with(&[Param::new("trust", "string", "code | open | off").req()]),
    Action::new("forget", "forget a paired device", Effect::Write).with(&[Param::new("id", "string", "a device id, or all").req()]),
    Action::new("open", "show or hide the Net window", Effect::Write).with(&[Param::new("on", "boolean", "")]),
];


const A_BATCH: &[Action] = &[
    Action::new("status", "every batch in a line, or one (`id`) with each command's result", Effect::Read)
        .with(&[Param::new("id", "string", "a batch id (b1): its commands' results in full")]),
    Action::new("run", "run `commands` in order, now or landing on the grid; `dry_run` validates and reports without running", Effect::Live)
        .with(&[
            Param::new("commands", "array", "an ordered list of {method, params, delay?}: each in any form the language takes (params beside the method, noun.verb)").req(),
            Param::new("atomic", "boolean", "check every command against the registry first and run none if any is invalid; then run all in one service pass. No `delay`"),
            Param::new("stop_on_error", "boolean", "stop at the first failed command (default true, false when atomic)"),
            Param::new("at", "string", "now (default) | next_beat | next_bar | next_phrase | bar:N: where the batch lands on the grid deck's own bar numbering"),
            Param::new("deck", "string", "the grid deck to time on (default the first of A, B that plays)"),
            Param::new("label", "string", "a name for batch status"),
            Param::new("dry_run", "boolean", "validate and report what each command would change; run nothing"),
        ])
        .probe(r#"{"commands":[]}"#),
    Action::new("cancel", "cancel a batch that is waiting or running: what has not run, does not", Effect::Write)
        .with(&[Param::new("id", "string", "the batch id (b1)").req()])
        .probe(r#"{"id":"__no_such_batch__"}"#),
];

const A_VOICE: &[Action] = &[
    Action::new("status", "the formant voices: presets, vowels, syllables, and the sung parts on each deck", Effect::Read),
    Action::new("presets", "the voice presets (name, summary, instrument)", Effect::Read),
    Action::new("vowel", "the vowel the voices on a deck sing, live (deck?, vowel: a e i o u ah oo or 0-6, net?)", Effect::Write)
        .with(&[
            Param::new("deck", "string", "A or B (default the focused deck)"),
            Param::new("vowel", "string|number", "a, e, i, o, u, ah, oo, or a position 0-6 (fractions morph)").req(),
            Param::new("net", "string", "one net (default every voice on the deck, e.g. vox)"),
        ])
        .probe(r#"{"vowel":"oo"}"#),
    Action::new("load", "compose a track with a sung part on a deck (genre, seed?, style: choir | robot | auto, preset?, structure?)", Effect::Write)
        .with(&[
            Param::new("deck", "string", "A or B (default the focused deck)"),
            Param::new("genre", "string", "default trance"),
            Param::new("seed", "integer", "default 1"),
            Param::new("style", "string", "choir | robot | auto (default: the preset's kind, else auto)"),
            Param::new("preset", "string", "a voice preset (robot, robot-choir, android, choir-aah, choir-ooh, soprano, bass-choir, talkbox)"),
            Param::new("structure", "string", "a song structure (default a loop)"),
        ])
        .probe(r#"{"genre":"trance","style":"choir"}"#),
    Action::new("vocoder", "the vocoder's synth sings: a voice preset as its carrier (preset, or off; carrier=true picks the synth as the carrier)", Effect::Write)
        .with(&[
            Param::new("preset", "string", "a voice preset, or off for the oscillator synth").req(),
            Param::new("carrier", "boolean", "also make the (singing) synth the carrier"),
        ])
        .probe(r#"{"preset":"robot-choir"}"#),
];

const A_VOCAL: &[Action] = &[
    Action::new("status", "the current line, how many are saved, the voices, where the phonemes come from", Effect::Read),
    Action::new("words", "how lyrics will sound: each word's phones, where they came from (dictionary or rules) and its syllables, as the voice will sing them", Effect::Read)
        .with(&[Param::new("lyrics", "string", "the words").req()])
        .probe(r#"{"lyrics":"Claude is on the road"}"#),
    Action::new("create", "fit lyrics to a key and a tempo (or to notes you give) and make it the current line; it is not saved or heard yet", Effect::Write)
        .with(&[
            Param::new("lyrics", "string", "the words: a comma, full stop or new line ends a phrase, _ is a rest, ro-bo-tic splits a word, digits are spelled").req(),
            Param::new("key", "string", "Bb minor, F# major, Am, 8A (default the form's, A minor)"),
            Param::new("bpm", "number", "the tempo (default the master's)"),
            Param::new("bars", "integer", "1 to 32 (default 2): sixteen syllables a bar at most"),
            Param::new("preset", "string", "a voice preset (default robot): robot, robot-choir, android, choir-aah, choir-ooh, soprano, bass-choir, talkbox"),
            Param::new("seed", "integer", "another seed, another tune (the rhythm stays)"),
            Param::new("name", "string", "default the first words"),
            Param::new("license", "string", "CC0-1.0 (default), CC-BY-4.0, all-rights-reserved"),
            Param::new("visibility", "string", "public (default) or private"),
            Param::new("author", "string", "who made it (default $DJ_AUTHOR)"),
            Param::new("notes", "array|string", "the notes to sing on, one per syllable in order: [{start,dur,midi}] in beats, or text \"0:1:A3 1:1:C4\" (start:dur:note)"),
        ])
        .probe(r#"{"lyrics":"I P O, Claude is on the road","key":"Bb minor","bpm":124,"bars":2}"#),
    Action::new("get", "the current line (or a saved one by id) with its notes and canonical JSON", Effect::Read)
        .with(&[Param::new("id", "string", "a saved line: cid, its head or tail, or a name")])
        .probe("!"),
    Action::new("list", "the saved lines (cid, name, lyrics, key, tempo, bars, license)", Effect::Read),
    Action::new("save", "keep the current line, content-addressed, with its license (CC0 and public unless said)", Effect::Write)
        .with(&[
            Param::new("name", "string", ""),
            Param::new("license", "string", "CC0-1.0, CC-BY-4.0 or all-rights-reserved"),
            Param::new("visibility", "string", "public or private"),
            Param::new("author", "string", ""),
        ])
        .probe("!"),
    Action::new("load", "make a saved line the current one (its form fields follow)", Effect::Write)
        .with(&[Param::new("id", "string", "cid, its head or tail, or a name").req()])
        .probe("!"),
    Action::new("license", "a line's license: with value, change it (a new CID); without, read it", Effect::Write)
        .with(&[Param::new("id", "string", "a saved line (default the current)"), Param::new("value", "string", "CC0-1.0, CC-BY-4.0, all-rights-reserved")])
        .probe("!"),
    Action::new("visibility", "public or private (a private line is never shared)", Effect::Write)
        .with(&[Param::new("id", "string", ""), Param::new("value", "string", "public or private")])
        .probe("!"),
    Action::new("fork", "a remix of a saved line (or the current one): it credits its parent, keeps the license, and becomes current", Effect::Write)
        .with(&[Param::new("id", "string", "default the current line"), Param::new("name", "string", ""), Param::new("author", "string", "")])
        .probe("!"),
    Action::new("share", "the object by CID, for publishing; a private line is refused", Effect::Read)
        .with(&[Param::new("id", "string", "default the current line")])
        .probe("!"),
    Action::new("delete", "remove a saved line", Effect::Write)
        .with(&[Param::new("id", "string", "").req()])
        .probe("!"),
    Action::new("play", "hear the current line once on the headphone cue (stop=true ends it); to hear it on the master, send it to_row play=true", Effect::Live)
        .with(&[Param::new("stop", "boolean", "stop the audition")])
        .probe(r#"{"stop":true}"#),
    Action::new("to_row", "render the current line to exactly its bars at its tempo and put it on the loop row (play=true starts it on the loop boundary)", Effect::Live)
        .with(&[Param::new("play", "boolean", "start it looping")])
        .probe("!"),
    Action::new("to_deck", "the current line as a looping sample on deck A or B, with its grid", Effect::Live)
        .with(&[Param::new("deck", "string", "A (default) or B")])
        .probe("!"),
    Action::new("to_vocoder", "the current line into the vocoder: as its modulator (the words shape the carrier; default) or its carrier", Effect::Live)
        .with(&[Param::new("role", "string", "modulator (default) or carrier")])
        .probe("!"),
    Action::new("hook", "add a vocal hook to what plays: lyrics fitted to a deck's track key (or key=) at the master tempo, rendered and looping on the loop row, a line of its own", Effect::Live)
        .with(&[
            Param::new("lyrics", "string", "the words").req(),
            Param::new("deck", "string", "A or B: take the key from that deck's track (default whichever has one)"),
            Param::new("key", "string", "override the key"),
            Param::new("bpm", "number", "override the tempo (default the master's)"),
            Param::new("bars", "integer", "1 to 32 (default 2)"),
            Param::new("preset", "string", "a voice preset (default robot)"),
            Param::new("seed", "integer", "another seed, another tune"),
            Param::new("play", "boolean", "start it looping (default true)"),
        ])
        .probe(r#"{"lyrics":"la la la la","key":"A minor","play":false}"#),
    Action::new("render", "write the current line as a WAV (mono, 32-bit float): one pass is exactly its bars long; seconds repeats it", Effect::Write)
        .with(&[
            Param::new("path", "string", "the .wav to write").req(),
            Param::new("sample_rate", "number", "default the engine's"),
            Param::new("seconds", "number", "repeat the loop to this length"),
        ])
        .probe("!"),
];

const A_KEY: &[Action] = &[
    Action::new("status", "both decks' keys (from the track itself, a hand-set tag, or the audio), in Camelot, how they mix, the transpose that would make them, and each deck's key lock", Effect::Read),
    Action::new("detect", "estimate a deck's key: a generated track answers the key it was composed in; a sample deck's audio is analysed (chroma against key profiles)", Effect::Read)
        .with(&[
            Param::new("deck", "string", "A or B (default: both)"),
            Param::new("profile", "string", "ensemble (default), krumhansl, temperley or albrecht"),
        ])
        .probe(r#"{"deck":"A"}"#),
    Action::new("set", "hand-set a deck's key (it wins over detection until the deck loads something else); null clears it", Effect::Write)
        .with(&[
            Param::new("deck", "string", "A or B").req(),
            Param::new("key", "string|null", "\"A minor\", \"Am\", \"8A\"").req(),
        ])
        .probe(r#"{"deck":"Z","key":"8A"}"#),
    Action::new("lock", "key lock on a deck: its tempo changes (key tempo) keep the pitch; a net deck is always locked", Effect::Write)
        .with(&[Param::new("deck", "string", "A or B").req(), Param::new("on", "boolean", "default: toggle")])
        .probe(r#"{"deck":"Z"}"#),
    Action::new("tempo", "set a deck's tempo; with key lock on a sample deck, a time-stretched copy (same pitch) is rendered off the window's thread and swapped in at the same place", Effect::Write)
        .with(&[Param::new("deck", "string", "A or B").req(), Param::new("bpm", "number", "20-400").req()])
        .probe(r#"{"deck":"Z","bpm":120}"#),
    Action::new("compat", "how two keys mix on the Camelot wheel, and the smallest transpose of the second that mixes", Effect::Read)
        .with(&[Param::new("a", "string", "a key").req(), Param::new("b", "string", "a key").req()])
        .probe(r#"{"a":"8A","b":"3B"}"#),
];

const A_MASHUP: &[Action] = &[
    Action::new("status", "the working mashup: sources (with key and tempo), parts, the mashup's key, clashes and their fixes, the CID and parent CIDs", Effect::Read),
    Action::new("sources", "the stem sources this build has (nets: generated tracks; demucs: external audio, a stub until demucs-rs)", Effect::Read),
    Action::new("new", "start an empty working mashup", Effect::Write)
        .with(&[Param::new("name", "string", "its name (default Mashup)")])
        .probe(r#"{"name":"probe"}"#),
    Action::new("add", "add a source track: genre:seed (techno:7, dnb:2602:B for the B riffs), deck:A (what a deck holds), or audio:<cid>", Effect::Write)
        .with(&[
            Param::new("spec", "string", "the track").req(),
            Param::new("label", "string", "what parts call it (default the next letter)"),
            Param::new("key", "string", "its key, for audio (generated tracks know theirs)"),
            Param::new("bpm", "number", "its tempo, for audio"),
        ])
        .probe(r#"{"spec":"nope:1"}"#),
    Action::new("remove", "remove a source and the parts that use it", Effect::Write)
        .with(&[Param::new("label", "string", "the source").req()])
        .probe(r#"{"label":"__none__"}"#),
    Action::new("stems", "a source's stems (drums, bass, harmony, melody, …)", Effect::Read)
        .with(&[Param::new("source", "string", "its label (default: every source)")]),
    Action::new("assign", "put a source's stem on a slot (replacing what the slot held)", Effect::Write)
        .with(&[
            Param::new("slot", "string", "drums, bass, melody, or any name (default: the stem's)"),
            Param::new("source", "string", "the source's label").req(),
            Param::new("stem", "string", "drums, bass, harmony, melody, or a net's id").req(),
        ])
        .probe(r#"{"source":"__none__","stem":"drums"}"#),
    Action::new("part", "a part's level (0-1), mute, solo, transpose (semitones) or remove", Effect::Write)
        .with(&[
            Param::new("slot", "string", "the part").req(),
            Param::new("level", "number", "0-1"),
            Param::new("mute", "boolean", ""),
            Param::new("solo", "boolean", ""),
            Param::new("transpose", "integer", "-12..12"),
            Param::new("remove", "boolean", "take the part out"),
        ])
        .probe(r#"{"slot":"__none__"}"#),
    Action::new("key", "the mashup's key (default: the first tonal part's source), and/or auto: move every clashing tonal part into harmony", Effect::Write)
        .with(&[Param::new("key", "string|null", "\"A minor\", \"8A\"; null: follow the parts"), Param::new("auto", "boolean", "fix the clashes")])
        .probe(r#"{"key":"H flat"}"#),
    Action::new("tempo", "the tempo every part plays at", Effect::Write)
        .with(&[Param::new("bpm", "number", "20-400").req()])
        .probe(r#"{"bpm":0}"#),
    Action::new("play", "realise the working (or a saved) mashup and put it on a deck: a playing deck swaps on its next bar, a stopped one loads and starts", Effect::Live)
        .with(&[Param::new("deck", "string", "A or B (default the focused deck)"), Param::new("id", "string", "a saved mashup")])
        .probe(r#"{"deck":"Z"}"#),
    Action::new("save", "save the working mashup under its content ID", Effect::Write)
        .with(&[Param::new("name", "string", "rename it first")])
        .probe(r#"{"name":""}"#),
    Action::new("load", "a saved mashup into the working one", Effect::Write)
        .with(&[Param::new("id", "string", "its cid, a part of it, or its name").req()])
        .probe(r#"{"id":"__no_such_mashup__"}"#),
    Action::new("list", "the saved mashups", Effect::Read),
    Action::new("fork", "the working mashup as a new one that points back at it (forkedFrom)", Effect::Write)
        .with(&[Param::new("name", "string", "the fork's name")])
        .probe(r#"{"name":""}"#),
    Action::new("delete", "delete a saved mashup", Effect::Write)
        .with(&[Param::new("id", "string", "its cid or name").req()])
        .probe(r#"{"id":"__no_such_mashup__"}"#),
    Action::new("open", "show or hide the Mashup window", Effect::Write).with(&[Param::new("on", "boolean", "default: toggle")]),
];

const A_RECORD: &[Action] = &[
    Action::new("status", "recording or not: seconds, frames, dropped frames, the tracklist so far, the file", Effect::Read),
    Action::new("start", "record the master output to a WAV, and the tracklist and replay from the action log, from now", Effect::Live)
        .with(&[Param::new("name", "string", "the set's name (default: the date)"), Param::new("max_minutes", "number", "stop by itself after this long (default: none, up to the 4 GB WAV limit)")])
        .probe(r#"{"max_minutes":0}"#),
    Action::new("stop", "stop: close the WAV, build the set (tracklist, replay, audio CID), sign it with the booth's key and save it; answers the set", Effect::Live)
        .with(&[Param::new("name", "string", "rename it first")])
        .probe(r#"{"name":"probe"}"#),
    Action::new("cancel", "stop and throw the take away", Effect::Write),
    Action::new("mark", "add a line to the live tracklist (a transition, a loop set, a note)", Effect::Write)
        .with(&[
            Param::new("kind", "string", "track, mashup, loopset or transition").req(),
            Param::new("label", "string", "in words"),
            Param::new("cid", "string", "its content ID"),
            Param::new("deck", "string", "A, B or none"),
        ])
        .probe(r#"{"kind":"__none__"}"#),
    Action::new("list", "the recorded sets, newest first", Effect::Read),
    Action::new("get", "a recorded set in full (default the newest)", Effect::Read)
        .with(&[Param::new("id", "string", "its cid, a part of it, or its name")])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("tracklist", "a set's tracklist as a cue sheet (default: the one recording now, else the newest)", Effect::Read)
        .with(&[Param::new("id", "string", "its cid or name")])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("verify", "check a saved set: the CID, the audio file against its CID, the signature against its did", Effect::Read)
        .with(&[Param::new("id", "string", "its cid or name").req()])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("replay", "play a recorded set again exactly: its arrangement through the action log's player", Effect::Live)
        .with(&[Param::new("id", "string", "its cid or name").req()])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("fork", "a recorded set as a new one to edit and perform again (no audio; forkedFrom points back), saved", Effect::Write)
        .with(&[Param::new("id", "string", "its cid or name").req(), Param::new("name", "string", "the fork's name")])
        .probe(r#"{"id":"__no_such_set__"}"#),
    Action::new("delete", "delete a recorded set and its audio", Effect::Write)
        .with(&[Param::new("id", "string", "its cid").req()])
        .probe(r#"{"id":"__no_such_set__"}"#),
];

pub(super) const NOUNS: &[Noun] = &[
    Noun {
        name: "status",
        tool: Some("djapp_status"),
        summary: "app, output, inputs, gear, controllers, clock, counters, notes",
        default_action: "",
        actions: &[],
        doc:
        "App status: version, uptime, the output device and rate, each input strip's device, \
         the gear plugged in (inpulse, apc, mpk, td3, pd705) and the controllers driving the \
         decks, the MIDI clock, counters (MIDI messages, unanswered controls), notes and the \
         TD-3 log. Start here.",
        schema: r#"{"type":"object","properties":{}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "state",
        tool: Some("djapp_state"),
        summary: "the live state: decks, mixer, transport, loops, inputs, vocoder, tricks (`section` picks one)",
        default_action: "",
        actions: &[],
        doc:
        "The live state: decks (mode, playing, spec, tempo, pitch, volume, EQ, sections, roles, \
         sample position, cues), mixer, transport (master tempo and its source), the loop row \
         and library, input strips (level, signal, channel, history mode/lag, capture). \
         section narrows it.",
        schema: r#"{"type":"object","properties":{"section":{"type":"string","enum":["all","decks","mixer","transport","loops","inputs"],"description":"default all"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "audio",
        tool: Some("djapp_audio"),
        summary: "audio devices and routing: list, switch the output, re-route an input strip",
        default_action: "devices",
        actions: &A_AUDIO,
        doc:
        "Audio devices and routing. devices: every output/input device the OS reports, the \
         current output, each strip's device. output: move the mixer to another output device \
         (part of its name; 'null' for none) while playing; it must run at the engine's rate. \
         input: feed an input strip from another device (device=null disconnects) and/or pick \
         its channel (L, R, LR).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["devices","output","input"]},"device":{"type":["string","null"],"description":"device name or part of it"},"input":{"type":["string","number"],"description":"input strip label (e.g. TD-3) or index"},"channel":{"type":"string","enum":["L","R","LR"]}},"required":["action"]}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "deck",
        tool: Some("djapp_deck"),
        summary: "one deck: transport, load, tempo, sync, sections, cues, loops, macros, sound, copy",
        default_action: "state",
        actions: &A_DECK,
        doc:
        "One deck (A or B; default the focused one). Actions: state, play (on?), pause, toggle, \
         stop, cue, load (genre, seed?, structure? | spec 'genre:seed:structure'), demo, file \
         (path: an audio file), next, seed_step (delta), tempo (bpm), pitch (value), \
         pitch_range (value), nudge (ratio), sync (align beat|bar), section (index), hot_cue / \
         clear_cue (slot), loop (op in|out|halve|double|exit|beats, beats), seek (seconds), \
         role (role, on), macro (id: a net macro), master_macro (id, bars?), panic, rewind, \
         focus, sound (palette: an id or null = the default | step: ±n along the playable ones; \
         neither lists them: the track's whole sound from a palette, the same song from the next \
         bar, kept for the deck's new tracks, logged as setPalette), copy (from, to: the \
         other deck onto this one by default, or this deck onto `to`; a net deck's spec, sound, \
         kit, palette and role mutes, or a sample deck's audio, landing on the source's next bar \
         at its tempo and place in the song; start=top: the net from its start, as a recorded \
         load). Returns the deck's state after.",
        schema: r#"{"type":"object","properties":{"deck":{"type":"string","enum":["A","B"]},"action":{"type":"string"},"on":{"type":"boolean"},"genre":{"type":"string"},"seed":{"type":"number"},"structure":{"type":"string","description":"loop (endless) or minimal|standard|extended|ab|drop|build|jam"},"spec":{"type":"string"},"path":{"type":"string"},"delta":{"type":"integer"},"bpm":{"type":"number"},"value":{"type":"number"},"ratio":{"type":"number"},"align":{"type":"string","enum":["beat","bar"]},"index":{"type":"integer"},"slot":{"type":"integer"},"op":{"type":"string"},"beats":{"type":"number"},"seconds":{"type":"number"},"role":{"type":"string"},"id":{"type":"string"},"bars":{"type":"integer"},"palette":{"type":["string","null"],"description":"sound palette id (studio, synth, synthwave, jazz, …); null = the default"},"step":{"type":"integer","description":"sound: move ±n palettes"},"from":{"type":"string","enum":["A","B"]},"to":{"type":"string","enum":["A","B"]},"start":{"type":"string","enum":["bar","top"]}},"required":["action"]}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "perform",
        tool: Some("djapp_perform"),
        summary: "any DeckAction (the session JSON) through Engine::perform",
        default_action: "",
        actions: &[],
        doc:
        "Any DeckAction through the engine's single perform entry, in its session JSON: \
         {op, deck, ...}. Ops include play, cue, hotCue, clearCue, loop, param (name: volume|\
         filter|gain|eqLow|eqMid|eqHigh|pitch|mute|reverbWet|reverbSize|reverbTone|autoGain, \
         value), crossfader, master, load, queueNext, nextTrack, autoAdvance, section, role, \
         roleDisable, macro, panic, stinger, sync, quantize, follow, grid, loopRecord, \
         grabBars, clip*, loop*, scene*, tapBars, voiceTweak, voiceSynth, lock, wander, \
         wanderPace, wanderLock, setKit, setPalette, setRoleVoice, mixOut, mixIn, rewind, focus, \
         stemMute, masterFx, fx, fxReset, fxKill, autoMix, autoMixNow. Answers done, stored, \
         ignored (why) or deferred.",
        schema: r#"{"type":"object","properties":{"action":{"type":"object","description":"the DeckAction, e.g. {\"op\":\"param\",\"deck\":\"A\",\"name\":\"eqLow\",\"value\":0.3}"}},"required":["action"]}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "transport",
        tool: Some("djapp_transport"),
        summary: "the transport and master tempo: start / stop / tap / lock / free tempo / auto",
        default_action: "state",
        actions: &A_TRANSPORT,
        doc:
        "The transport and master tempo (what the MIDI clock follows): state, start, stop, \
         toggle, tap (tap tempo), lock (deck: lock the master tempo to it), unlock, free_bpm \
         (bpm: the tempo when no deck plays), auto (on: true / false / toggle; default toggle). \
         Auto (on by default, config transport.auto): a deck that starts playing starts the \
         clock locked to it (Start on its bar line), the master deck keeps the lock as the \
         crossfader moves (no Stop / Start), the last deck stopping stops it. A start, stop, \
         toggle, lock or unlock by hand holds auto off until a deck next plays (auto.held).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["state","start","stop","toggle","tap","lock","unlock","free_bpm","auto"]},"deck":{"type":"string"},"bpm":{"type":"number"},"on":{"type":["boolean","string"],"description":"action=auto: true, false or toggle"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "mixer",
        tool: Some("djapp_mixer"),
        summary: "crossfader, master, headphones, a strip's volume / gain / EQ / PFL / mute / aux send",
        default_action: "",
        actions: &[],
        doc:
        "Set any of: crossfader, master, headphone_mix, headphone_level (0..1); and for deck \
         (A, B, or L the loop row): volume, gain, filter, low, mid, high (strip knobs), pfl, \
         mute, aux (its aux send, docs/AUX.md). Returns the mixer state. Omit everything to just read it.",
        schema: r#"{"type":"object","properties":{"crossfader":{"type":"number"},"master":{"type":"number"},"headphone_mix":{"type":"number"},"headphone_level":{"type":"number"},"deck":{"type":"string","enum":["A","B","L"]},"volume":{"type":"number"},"gain":{"type":"number"},"filter":{"type":"number"},"low":{"type":"number"},"mid":{"type":"number"},"high":{"type":"number"},"pfl":{"type":"boolean"},"mute":{"type":"boolean"},"aux":{"type":"number","description":"the strip's aux send"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "fx",
        tool: Some("djapp_fx"),
        summary: "the FX unit on a deck: set, kill, reset, params",
        default_action: "",
        actions: &A_FX,
        doc:
        "The FX unit on a deck (A, B, or both when omitted): set (param, value), kill (on), \
         reset, params (list the ids).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["set","kill","reset","params"]},"deck":{"type":"string"},"param":{"type":"string"},"value":{"type":"number"},"on":{"type":"boolean"}},"required":["action"]}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "aux",
        tool: Some("djapp_aux"),
        summary: "the aux send / return bus and the macros over it",
        default_action: "",
        actions: &[],
        doc:
        "The aux send/return bus (docs/AUX.md): every strip (A, B, L, each input) has an aux \
         send (strip + send, 0..1, post-fader) into a room and a ping-pong echo, which come \
         back on the FX return (return 0..1, mute). The six effect sliders are beats' ids \
         (reverb-size, reverb-damp, reverb-wet 0..100; delay-time 1..100, delay-feedback 0..90, \
         delay-wet 0..100); reset puts them at beats' defaults. macro fires one of the eleven \
         pan / shape / aux macros (ping-pong, hard-left, hard-right, auto-pan, mono, \
         delay-throw, dub-delay, cathedral, tighten, loosen, pulse) at a deck (default the \
         focused one) for bars bars, on its next bar; cancel puts them back (deck, or all). \
         Returns the bus: sends, return, sliders, the macros running.",
        schema: r#"{"type":"object","properties":{"strip":{"type":"string","description":"A, B, L or an input's label"},"send":{"type":"number"},"return":{"type":"number"},"mute":{"type":"boolean"},"reverb-size":{"type":"number"},"reverb-damp":{"type":"number"},"reverb-wet":{"type":"number"},"delay-time":{"type":"number"},"delay-feedback":{"type":"number"},"delay-wet":{"type":"number"},"reset":{"type":"boolean"},"macro":{"type":"string","enum":["ping-pong","hard-left","hard-right","auto-pan","mono","delay-throw","dub-delay","cathedral","tighten","loosen","pulse"]},"deck":{"type":"string"},"bars":{"type":"integer"},"cancel":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "input",
        tool: Some("djapp_input"),
        summary: "an input strip: trim, volume, mute, PFL, EQ, channel, latency (or measure it), crossfader side",
        default_action: "",
        actions: A_INPUT,
        doc:
        "An audio input strip (the gear's audio back in): set any of trim_db, volume, mute, pfl, \
         filter, low, mid, high, channel (L|R|LR), latency_ms (how late captures read). \
         Returns the strip. calibrate_latency (seconds?, apply?, min_confidence?) measures how \
         late a source that is slaved to the master clock arrives: it finds the source's beats \
         (Beat This! when the models are there, else the DSP detector) in the recording and \
         compares them with the clock's, answers the suggested latency_ms with its confidence, \
         and with apply=true sets it when confident. The work runs off the audio thread; the \
         call answers at once and the result is `latency`. Also named `inputs`.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","calibrate_latency","latency"]},"seconds":{"type":"number"},"apply":{"type":"boolean"},"min_confidence":{"type":"number"},"input":{"type":["string","number"],"description":"label (TD-3, PD705) or index; default the jog-aimed / first"},"trim_db":{"type":"number"},"volume":{"type":"number"},"mute":{"type":"boolean"},"pfl":{"type":"boolean"},"filter":{"type":"number"},"low":{"type":"number"},"mid":{"type":"number"},"high":{"type":"number"},"channel":{"type":"string","enum":["L","R","LR"]},"latency_ms":{"type":"number"}}}"#,
        params: &[],
        aliases: &["inputs"],
        local: false,
    },
    Noun {
        name: "loops",
        tool: Some("djapp_loops"),
        summary: "the loop row and the capture library",
        default_action: "list",
        actions: &A_LOOPS,
        doc:
        "The loop row (L) and the capture library. list, play / stop (id or slot: play only \
         starts a clip that is off or stopping, stop only stops one that is in or starting; \
         returns changed and state), stop_all, \
         remove, record (source, on, bars?: hold LOOP on a source, whole bars; a quick tap \
         with bars takes exactly that many), grab (source, bars: the last bars, default 4). A \
         source is a deck (A, B), V (the vocoder strip, after its fader; pre=true before it), \
         M (the master output) or an input strip's label (TD-3, PD705); default the focused \
         deck (`deck` is accepted for `source`). The take lands on the row and, from V, M or \
         an input, in the library. volume (value), mute (on), library (saved loops), library_load (id), \
         library_save (id or slot), library_delete (id), library_row (put the library on the \
         row). to_deck (deck A|B, id or slot; neither = the newest clip): move a row clip onto \
         a deck (it leaves the loop channel and plays there in time with the other deck). \
         library_to_deck (id, deck): a library loop onto a deck, via the row. root (op: status | \
         set (pitch: C, F#, Bb or 0-11; slot or the playing loops) | detect (look at the audio \
         again, letting go of a hand's root) | auto (on)): the pitch class keys transpose a \
         loop from; found from the file name (Am, F#min) or the audio, a hand wins.",
        schema: r#"{"type":"object","properties":{"op":{"type":"string","enum":["status","set","detect","auto"]},"pitch":{"type":["string","integer"]},"action":{"type":"string"},"id":{"type":"string"},"slot":{"type":"integer"},"deck":{"type":"string"},"source":{"type":"string","description":"A, B, V, M or an input label"},"pre":{"type":"boolean","description":"V before its fader"},"on":{"type":"boolean"},"bars":{"type":"integer"},"value":{"type":"number"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "keys",
        tool: Some("djapp_keys"),
        summary: "where the controllers' piano keys go",
        default_action: "status",
        actions: &A_KEYS,
        doc:
        "Where the controllers' piano keys go (the MPK's). status (default): mode, deck, quant, \
         held keys. mode (mode: live = play the deck's lead voice, locked to the song's scale; \
         transpose = transpose the loops; td3 = play the TD-3; toggle = live <-> transpose), deck \
         (A | B | aimed | next), quant (now | beat | bar: held back to the hub clock's next beat / \
         bar), scale_lock (on), note (note, velocity?, down?, deck?, quant?: play one key as a \
         controller does, logged as a replayable op), off (let go of everything).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","mode","deck","quant","scale_lock","note","off"]},"mode":{"type":"string","enum":["live","transpose","td3","toggle"]},"deck":{"type":"string"},"quant":{"type":"string","enum":["now","beat","bar"]},"on":{"type":"boolean"},"note":{"type":"integer"},"velocity":{"type":"integer"},"down":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "pads",
        tool: Some("djapp_pads"),
        summary: "the MPK's pads: mode, learn, notes",
        default_action: "status",
        actions: &A_PADS,
        doc:
        "The MPK's pads. status (default): the mode seen (PROG = Program Change, bank B repeats \
         bank A; NOTE = 16 distinct pads), the on-screen notice, the notes in force and whether \
         they were learned. learn: record the 16 pad notes as they are pressed (bank A 1-8, then \
         bank B 1-8; the unit must be in NOTE mode) and write them into the device map; poll \
         status for progress. cancel, clear (forget what was learned), set (channel 1-16, notes: \
         by hand), feed (bytes: a message as if the MPK sent it).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","learn","cancel","clear","set","feed"]},"channel":{"type":"integer"},"notes":{"type":"array","items":{"type":"integer"}},"bytes":{"type":"array","items":{"type":"integer"}}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "vox",
        tool: Some("djapp_vox"),
        summary: "the vocal pads: folder, page, choke, stretch, play",
        default_action: "status",
        actions: &A_VOX,
        doc:
        "The vocal pads: one-shots from a folder (default vox/ in the loop library; the \
         \"vocals\" binding preset puts them on the MPK's pads, bank A the first 8 of the page, \
         bank B the next 8), on the beat, one choke group, stretched to the master tempo when the \
         file's tempo is known (its name, the pad's bpm or vox.bpm). status (default), dir (dir: \
         a folder, or none for the default), scan, page (op: next | prev | first), set (choke, \
         stretch, quant, bpm), play (pad 1-16).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","dir","scan","page","set","play"]},"dir":{"type":["string","null"]},"op":{"type":"string","enum":["next","prev","first"]},"choke":{"type":"string"},"stretch":{"type":"boolean"},"quant":{"type":"string","enum":["now","beat","bar"]},"bpm":{"type":"number"},"pad":{"type":"integer"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "capture",
        tool: Some("djapp_capture"),
        summary: "capture to loop from an input's history",
        default_action: "grab",
        actions: &A_CAPTURE,
        doc:
        "Capture to loop from an input strip's history onto the loop row (saved to the \
         library): grab (beats?: the last N beats, default the strip's loop length), start (a \
         live take from the next bar), end (on the bar after), arm (on), status. bars (bars?, \
         anchor?): a loop of exactly N bars cut on bar lines of the input's beat grid (the \
         history's, whether or not the player is on the clock), from the head: scrubbed back, the \
         bars from the bar line nearest the head on (anchor start) or the bars before it (end); \
         live, the last N whole bars. Its tempo is the music's own, so it follows the master. \
         Needs a grid: `analysis beats target=input` (it waits for one for a moment).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["grab","start","end","arm","status","bars"]},"input":{"type":["string","number"]},"beats":{"type":"number"},"on":{"type":"boolean"},"bars":{"type":"integer"},"anchor":{"type":"string","enum":["start","end"]}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "history",
        tool: Some("djapp_history"),
        summary: "any strip's history: scrub, live, loop, window, jog aim",
        default_action: "status",
        actions: &A_HISTORY,
        doc:
        "Any strip's history (an input's: the last N beats of the gear's audio; or deck A / B, \
         L the loop row, V the vocoder, M the master, each keeping the same length): strip \
         (or input) names it, default the jog-aimed one. status, touch (hold \
         the head), scrub (beats or seconds: negative moves back in time, positive toward live; \
         the head holds there until release or live), release, live (back to the live \
         input), loop (beats?: loop the last beats in place), loop_step (dir +1 doubles / -1 \
         halves), beats (an input's window length), jog (deck A|B|off: aim that deck's jog \
         wheel at this history; it scrubs it while the deck is empty or SHIFT / historyJog is \
         held, else the deck's loop), snap (deck?: the history a jog is aimed at, back to live; \
         also a historySnap binding). With a beat grid (an input's, from `analysis`): grid \
         (tempo, bar 1, confidence, the bar lines in the window, where the head is), align \
         (unit bar|beat, steps?: the head onto the nearest line or `steps` lines on), loop_bars \
         (bars: loop exactly N bars in place, ending on a bar line). With analysis.snap on a \
         released jog scrub lands on a line by itself.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","touch","scrub","release","live","loop","loop_step","beats","jog","snap","grid","align","loop_bars"]},"unit":{"type":"string","enum":["bar","beat"]},"steps":{"type":"integer"},"bars":{"type":"integer"},"strip":{"type":"string","description":"an input's label, A, B, L, V or M"},"input":{"type":["string","number"]},"beats":{"type":"number"},"seconds":{"type":"number"},"dir":{"type":"integer"},"deck":{"type":"string"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "clock",
        tool: Some("djapp_clock"),
        summary: "the MIDI clock master, PD705 steps, the TD-3's clock source",
        default_action: "",
        actions: &[],
        doc:
        "The MIDI clock master (it follows the transport and master tempo): read it, or set \
         td3_clock (send clock + Start/Stop to the TD-3), td3_offset_ms, pd705_steps (play the \
         PD705's steps on the clock), pd705_kit (factory | acid-house: \
         the kit on the pad; its loop pads are never played by the steps or loop routes), \
         pd705_loop_pads (custom loop pads, 1-15), td3_source_auto (USB clock while the hub is master), \
         td3_source (usb | internal | read: the TD-3's clock-source SysEx).",
        schema: r#"{"type":"object","properties":{"td3_clock":{"type":"boolean"},"td3_offset_ms":{"type":"number"},"pd705_steps":{"type":"boolean"},"pd705_kit":{"type":"string","enum":["factory","acid-house"]},"pd705_loop_pads":{"type":"array","items":{"type":"integer","minimum":1,"maximum":15}},"td3_source_auto":{"type":"boolean"},"td3_source":{"type":"string","enum":["usb","internal","read"]}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "td3",
        tool: Some("djapp_td3"),
        summary: "the TD-3: keys, pattern editor, audition, write (dry run unless confirmed)",
        default_action: "status",
        actions: &A_TD3,
        doc:
        "The TD-3: pd705_steps (on | off | state, with any action: the hub plays the PD705's \
         groove on the master clock, since the pad ignores MIDI clock; also djapp --pd705-steps), \
         status, keys (on: the MPK's keys play it), octave (value), channel (value \
         1-16), generate (seed?, root?: a new acid pattern in the editor), slot (slot e.g. \
         4-16), step (step 0-15 with note C..B|C^, transpose down|none|up, accent, slide, tie, \
         rest; or active: steps that play: the editor's grid), audition (step: play one step now), write (send the pattern to the slot via \
         td3-midi: a dry run unless confirm=true; stop the transport first), cutoff_cc (cc: the \
         CC a cutoff listens to, for the td3_cutoff trick; null = none, the stock TD-3's).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","keys","octave","channel","generate","slot","step","audition","write","cutoff_cc"]},"cc":{"type":["integer","null"]},"note":{"type":["string","integer"]},"transpose":{"type":"string","enum":["down","none","up"]},"accent":{"type":"boolean"},"slide":{"type":"boolean"},"tie":{"type":"boolean"},"rest":{"type":"boolean"},"active":{"type":"integer"},"pd705_steps":{"type":["string","boolean"],"enum":["on","off","state",true,false]},"on":{"type":"boolean"},"value":{"type":"integer"},"seed":{"type":"number"},"root":{"type":"integer"},"slot":{"type":"string"},"step":{"type":"integer"},"confirm":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "kits",
        tool: Some("djapp_kits"),
        summary: "the PD705 kit library: kits, pads, patterns, build, install",
        default_action: "status",
        actions: &A_KITS,
        doc:
        "The PD705 kit library (docs/HUB.md M8, the Kits window). A kit is samples on the pad's \
         inputs plus its own step pattern. status (default: kits, the tracker, the hold-back, \
         jobs, the last build / install, a log tail), list, get (kit: slots, loop pads, the step \
         rows), add / remove / rename (to) / duplicate (to) / select (kit), host (the Pi, \
         user@host), pad (pad 1-15|pad7|trig1|foot2 with file: a WAV path, or clip / slot: a \
         loop-row clip, exported; name, volume 0-127, loop true|false|auto; clear: true takes \
         it off), pattern (op get | step (pad, step 0-15.., velocity 0-127, 0 clears) | cycle \
         (pad, step: soft/medium/hard/rest) | bars 1-4 | preset (house|breaks|halftime|random \
         |empty, seed) | default | set (pattern: JSON)), build (kit?: pd705 build, shows the \
         sha256), install (kit?: a DRY RUN unless confirm=true; it builds, asks the installer \
         for its plan and checks the plan names the same sha256, then for a real one copies to \
         the Pi stick and reads the installer's own sha256 check; refused while the hub plays \
         the pad unless confirm_live=true; wait=true holds up to 30 s for the answer), loaded \
         (the staged kit is on the pad now: after LOAD > Kit > Append on the pad), mark_loaded \
         (kit: it is on the pad some other way), forget (all?), track (on | off | state: the \
         hold-back follows the installed kit), audition (pad: plays it on the PD705 now), \
         open. A real install detaches the Pi's USB stick for a few seconds: never run one \
         unless asked.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","list","get","add","remove","rename","duplicate","select","host","pad","pattern","build","install","loaded","mark_loaded","forget","track","audition","open"]},"kit":{"type":"string"},"to":{"type":"string"},"host":{"type":"string"},"pad":{"type":["integer","string"]},"file":{"type":"string"},"clip":{"type":"string"},"slot":{"type":"integer"},"name":{"type":"string"},"volume":{"type":"integer","minimum":0,"maximum":127},"loop":{"type":["boolean","string"]},"clear":{"type":"boolean"},"op":{"type":"string","enum":["get","step","cycle","bars","preset","default","set"]},"step":{"type":"integer","minimum":0},"velocity":{"type":"integer","minimum":0,"maximum":127},"bars":{"type":"integer","minimum":1,"maximum":4},"preset":{"type":"string","enum":["house","breaks","halftime","random","empty"]},"seed":{"type":"number"},"pattern":{"type":"object"},"confirm":{"type":"boolean","description":"install: really install (default is a dry run)"},"confirm_live":{"type":"boolean"},"wait":{"type":"boolean"},"on":{"type":["boolean","string"]},"tab":{"type":"string","enum":["pads","steps"]},"all":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "midi",
        tool: Some("djapp_midi"),
        summary: "gear ports, the monitor, send a message, a midiOut",
        default_action: "",
        actions: &A_MIDI,
        doc:
        "MIDI to and from the gear. ports: the gear catalog with ports present, unknown ports, \
         the controllers driving. monitor (port?, n?): the latest messages per input port, in \
         words; paused (on/off: the monitor window's pause), clear (true: empty it). send (device: td3|pd705|part of a port name, bytes: [status, data...]): one \
         channel-voice message or Start/Continue/Stop (SysEx refused). out (device, args: a \
         midiOut binding's args, e.g. {kind: note, note: 36, velocity: 100, ms: 120}).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["ports","monitor","send","out"]},"device":{"type":"string"},"port":{"type":"string"},"n":{"type":"integer"},"paused":{"type":"boolean"},"clear":{"type":"boolean"},"bytes":{"type":"array","items":{"type":"integer"}},"args":{"type":"object"},"index":{"type":"integer"},"value":{"type":"number"},"pressed":{"type":"boolean"}},"required":["action"]}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "bindings",
        tool: Some("djapp_bindings"),
        summary: "devices, maps, presets, bind / unbind, press a control, the target registry",
        default_action: "devices",
        actions: &A_BINDINGS,
        doc:
        "The controllers' bindings (MPK, APC, Inpulse). devices: each with its port, presets \
         and overrides. list (device, group?): its groups, effective layout and overrides. map \
         (device): bound / total. preset (preset: next|prev|name, on every device). bind \
         (device, binding: a layout entry {group, index?, action, deck?, slot?, shift?, role?, \
         macro?, param?, args?}) and unbind (device, group, index?, shift?) in the live preset, \
         saved. preset_add (device, name: a copy of the live preset, made live) and preset_delete \
         (device: the live preset). press (device, group, index, shift?): press a note control as if touched. \
         targets (family?): every bindable target with its params.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["devices","list","map","preset","preset_add","preset_delete","bind","unbind","press","targets","inject","jog","view","add_device","import","keys","activity"]},"device":{"type":"string"},"name":{"type":"string"},"group":{"type":"string"},"index":{"type":"integer"},"shift":{"type":"boolean"},"preset":{"type":"string"},"binding":{"type":"object"},"family":{"type":"string"}},"required":["action"]}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "target",
        tool: Some("djapp_target"),
        summary: "fire any registry target, as a bound control would",
        default_action: "",
        actions: &[],
        doc:
        "Fire any binding target as a bound control would (ids from djapp_bindings \
         action=targets: captureGrab, historyScrub, td3ClockSource, midiOut, mapPreset, \
         oneShot, loopFile, transpose, inputLatency, ...). deck, slot, index (a key's note), \
         value (0..1), pressed (default true), release (also send the release), args (the \
         binding's args) and role/macro/param/input/source/preset/device/beats shortcuts.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"},"deck":{"type":"string"},"slot":{"type":"integer"},"index":{"type":"integer"},"value":{"type":"number"},"pressed":{"type":"boolean"},"release":{"type":"boolean"},"args":{"type":"object"}},"required":["id"]}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "route",
        tool: Some("djapp_route"),
        summary: "loop routing: a loop's parts out to gear as notes while it plays",
        default_action: "status",
        actions: &A_ROUTE,
        doc:
        "Loop routing: while a loop plays, send its parts (melody, bass, harmony, arp, drums, \
         kick, snare, hihat, clap, all) out as notes to dests td3, pd705, synth:A / synth:B (a \
         deck's live voice) or midi:<part of a port name>[@channel], several at once. status: \
         every clip on the row with its notes (ready with parts and counts, or none with why: \
         an audio-only loop has no notes) and routes, the dests and whether connected, the \
         log. set (id or slot; neither = every loop playing now, part, dest, on: default true) \
         and toggle route one part; clear (id/slot, or everything). notes (id/slot, part?, n?) \
         lists a clip's notes in sixteenths. offset (ms: moves every dest but the TD-3, which \
         uses the clock's TD-3 offset). add_dest (dest). open (on): the Route panel.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","set","toggle","clear","notes","offset","add_dest","open"]},"id":{"type":"string"},"slot":{"type":"integer"},"part":{"type":"string"},"dest":{"type":"string"},"on":{"type":"boolean"},"n":{"type":"integer"},"ms":{"type":"number"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "log",
        tool: Some("djapp_log"),
        summary: "the action log and its beat-stamped arrangement: save / load / play a passage",
        default_action: "lines",
        actions: &A_LOG,
        doc:
        "The action log, for composing from a jam: every move made on djapp (controllers, \
         window, socket) as words AND as session-JSON actions stamped with bar, beat and \
         tempo. lines (n?, since?: a seq): the log as people read it, with bar markers, \
         beats since the last move and a wheel's pattern of moves (forward 2, back 1). \
         arrangement (from_bar?, to_bar?): the recording, or bars of it, as an arrangement \
         JSON (schema djrust.arrangement/1: decks it starts on + events {at beats, bar, \
         beat, bpm, action: a DeckAction or {op: swap, deck, source, start}}), editable. \
         save (from_bar?, to_bar?, path?): write it to a file. load (path). play (path, or \
         arrangement: an edited one inline, or from_bar + to_bar of the log): replay its \
         moves from the next bar at the tempo running now. stop, clear, open (on).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["lines","arrangement","save","load","play","stop","clear","open"]},"n":{"type":"integer"},"since":{"type":"integer"},"from_bar":{"type":"integer"},"to_bar":{"type":"integer"},"path":{"type":"string"},"arrangement":{"type":"object"},"on":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "screenshot",
        tool: Some("djapp_screenshot"),
        summary: "a PNG of the window",
        default_action: "",
        actions: &[],
        doc:
        "A PNG of djapp's window as it is now (the window stays open). Returns the image.",
        schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"where to write it (default /tmp/djapp-<pid>.png)"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "clips",
        tool: Some("djapp_clips"),
        summary: "the loop row in full: scenes, crops, sequences, retune, clip FX, per-clip launch settings",
        default_action: "list",
        actions: &A_CLIPS,
        doc:
        "The loop row in full and its edits. list (default): the row's settings (launch \
         quantum, fade, tempo follow, key lock, FX link, clock bpm), every clip (state, gain, \
         mute, solo, its own key_lock / quantum / fade / follow (null = the row's), clip FX, \
         sequence steps, colour), the scenes and the clips removed this run. The other actions \
         are the session DeckActions of the same name, params passed through: scene_save \
         {name}, scene_launch {name}, scene_rename {name,to}, scene_delete {name}, crop \
         {id,from,to} (beats; the crop takes the clip's place), sequence \
         {steps:[{clip,repeats}],replacing?}, retune {id,voices:{\"<channel>\":\"<instrument>\"}}, \
         fx {id,fx:{bypass,drive,mode,amount,bits,downsample,tone,mix,trim,autoLevel}}, \
         fx_link {on}, key_lock {id,on}, row_key_lock {on}, quantize {id,beats|null}, \
         row_quantize {beats}, fade {id,fade: off|10ms|beat|bar|null}, row_fade {fade}, \
         follow {id,mode: source|louder|null}, row_follow {mode}, gain {id,value}, rename \
         {id,title}, color {id,color}, restore {id,at?}, forget {id}. Playing a clip onto deck \
         A/B is not this tool's.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["list","scene_save","scene_launch","scene_rename","scene_delete","crop","sequence","retune","fx","fx_link","key_lock","row_key_lock","quantize","row_quantize","fade","row_fade","follow","row_follow","gain","rename","color","restore","forget"]},"id":{"type":"string","description":"clip id (from list)"},"name":{"type":"string"},"to":{"type":"string"},"from":{"type":"integer"},"beats":{"type":["integer","null"]},"on":{"type":"boolean"},"value":{"type":"number"},"title":{"type":"string"},"color":{"type":["string","null"]},"fade":{"type":["string","null"]},"mode":{"type":["string","null"]},"fx":{"type":"object"},"steps":{"type":"array"},"replacing":{"type":"string"},"voices":{"type":"object"},"at":{"type":"integer"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "wander",
        tool: Some("djapp_wander"),
        summary: "the walk on each deck: status, on / off, pace, lock, grab",
        default_action: "status",
        actions: &A_WANDER,
        doc:
        "The walk (Wander): a net deck that moves on its own each phrase (key, palette, kit, \
         voices, layers, stems, Hits). status (default): each deck's walk (on, pace, paused, \
         step, locks, countdown in bars, why it is idle). on / off {deck}, pace {deck,bars}, \
         lock {deck,dimension: genre|tempo|key|structure|instruments|arrangement|fx|palette|layers, on}, grab {deck}: as a turn of the browser wheel, pausing a \
         wandering deck until a phrase passes with no turn.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","on","off","pace","lock","grab"]},"deck":{"type":"string","description":"A or B (default: the focused deck)"},"bars":{"type":"integer"},"dimension":{"type":"string"},"on":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "automix",
        tool: Some("djapp_automix"),
        summary: "auto-mix: status, set, off, now, defer",
        default_action: "status",
        actions: &A_AUTOMIX,
        doc:
        "Auto-mix: hands the music from one deck to the other on its own. status (default): \
         its state and one-line status. set {on,bars,every}, off, now (start a hand-off now), \
         defer {handoff} (put off the hand-off numbered handoff, as the engine itself does).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","set","off","now","defer"]},"on":{"type":"boolean"},"bars":{"type":"integer"},"every":{"type":"integer"},"handoff":{"type":"integer"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "autonext",
        tool: Some("djapp_autonext"),
        summary: "auto-next every N bars on a deck",
        default_action: "status",
        actions: &A_AUTONEXT,
        doc:
        "Auto-next every N bars on deck A or B (beats.bitwrap.io's Auto-DJ Regen, per deck): \
         the deck steps to its next seed by itself, exactly as one click of the browser wheel \
         (seed + 1, same song settings), landing on the N-bar line of its own grid (the one the \
         master clock follows). action status (default: every deck's on, bars, bars_left, \
         idle reason), on, off, toggle, set {bars}, up / down (the next of 4 8 16 32 64). \
         deck defaults to the focus; bars 1-1024. Logged in the action log, each step with the \
         seed it landed on. Bindable as `autoNextBars`.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","on","off","toggle","set","up","down"]},"deck":{"type":"string"},"bars":{"type":"integer"},"on":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "trick",
        tool: Some("djapp_trick"),
        summary: "performance tricks on a deck: on / off / toggle a kind",
        default_action: "status",
        actions: &A_TRICK,
        doc:
        "Performance tricks on deck A or B (docs/TRICKS.md), each an insert on the deck's strip: \
         kind stutter (beat repeat: div 4|8|16|32 (6|12|24 triplets), rise semitones per \
         repeat), pump (sidechain duck from a deck's kick: source auto|grid|A|B, amount = \
         depth 0-1, div = release as a note division), delay (dub delay throw: the deck is sent in while on, \
         the echoes ring after off; beats = time (0.75 dotted 1/8), feedback 0-0.95, amount = \
         send), wash (reverb wash-out: the dry fades out over `beats` into a big room at \
         `amount`; off brings the dry back and the room decays), verb_freeze (catches `beats` of \
         the deck in the room and holds it near-forever under the deck at `amount`; off lets \
         it decay), tape_stop (the deck slows to a halt over `beats`, silent until off), \
         spin_back (whipped backwards at up to 4 x `amount`, easing to a halt over `beats`, \
         silent until off), half_time / double_time (the deck's clock at 0.5x / 2x its tempo, on \
         the beat; a net deck's notes keep their pitch, a sample deck varispeeds; off lands it \
         back in phase with where it would have been), filter (the one-knob resonant filter: on \
         rides it to amount -1 (low-pass) .. 1 (high-pass) over `bars` from the bar, res = \
         resonance 0-1; off rides back to open over a beat; action knob + value 0-1 sets it by \
         hand, as a trickKnob binding does), td3_accent / td3_slide (the notes the hub sends \
         the TD-3, from the MPK's keys and routed loops, go out accented (velocity 127) / slid \
         (legato); deck is ignored), td3_cutoff (rides a cutoff CC to amount 0-1 over `bars` \
         from the bar, off back to the middle; refused for the stock TD-3, which has no \
         cutoff CC: set one with djapp_td3 cutoff_cc for gear that has it), riser (a noise \
         riser for transitions: white noise through a band-pass sweeping 300 Hz -> 12 kHz over \
         `bars` (4) from the bar, rising to `amount` (0.5), cut at its end; off cuts it early). action on / off / toggle, or status (default: every deck's tricks). A fire \
         lands on the deck's grid (the trick's own by default; quant now|32nd|16th|8th|beat|bar \
         or a number of beats). Logged in the action log. Pads do the same with a `trick` \
         binding, momentary or latched.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","on","off","toggle","knob"]},"deck":{"type":"string"},"kind":{"type":"string","enum":["stutter","pump","delay","wash","verb_freeze","tape_stop","spin_back","half_time","double_time","filter","td3_accent","td3_slide","td3_cutoff","riser"]},"res":{"type":"number"},"value":{"type":"number","description":"knob: 0 low-pass all the way, 0.5 open, 1 high-pass all the way"},"beats":{"type":"number"},"feedback":{"type":"number"},"source":{"type":"string","description":"pump: auto (the other deck's kick if it plays, else the grid), grid, A or B"},"quant":{"type":"string"},"div":{"type":"number"},"rise":{"type":"number"},"amount":{"type":"number"},"bars":{"type":"number"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "moves",
        tool: Some("djapp_moves"),
        summary: "the Perform window's DJ moves on the phrase grid",
        default_action: "status",
        actions: &A_MOVES,
        doc:
        "The Perform window's DJ moves (docs/PERFORM.md), each armed now and landing on the \
         bar / phrase grid the decks keep: bass_swap (the incoming `deck`'s lows cut now, swapped \
         with the other deck's on the next `bars`-bar phrase line, 8|16|32), phrase_sync (on the \
         next `bars` 8|16|32 line: `do` seed = the deck's next seed, composed a bar early so it \
         comes in on the line; `do` cut = the crossfader over to the deck), filter_exit (a \
         high-pass ride over `bars` from the next bar, then the channel muted on the downbeat), \
         echo_out (reverb wash on the one: the dry gone in a 16th, the room ringing `bars`, then \
         the channel muted), loop_roll (a sample deck: a 4-beat loop halved every `bars` 4 -> 2 \
         -> 1 -> 1/2 beat, released on the line after; a net deck rolls a beat repeat 1 -> 1/8), \
         vocoder_handoff (carrier and modulator swapped on the next phrase line of `deck`'s \
         grid). action status (default: what is armed / running, the last word on each, and the \
         harmonic hint: both decks' keys and whether they mix: same, fifth, relative, clash), \
         arm, cancel, toggle, cancel_all. A stopped grid fires at once.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","arm","cancel","toggle","cancel_all"]},"kind":{"type":"string","enum":["bass_swap","phrase_sync","filter_exit","echo_out","loop_roll","vocoder_handoff"]},"deck":{"type":"string"},"bars":{"type":"integer"},"do":{"type":"string","enum":["seed","cut"]}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "autodj",
        tool: Some("djapp_autodj"),
        summary: "Auto-DJ: move on by itself every N bars, with effects across the handover",
        default_action: "status",
        actions: &A_AUTODJ,
        doc:
        "Auto-DJ (docs/AUTO-DJ.md; beats.bitwrap.io's Auto-DJ, for two decks): every `bars` bars \
         (8-64) on a phrase line it hands over to the next thing, with effects automated across \
         the transition. A loop-set pair (group g on both decks) goes to group g + 1, wrapping; \
         two playing decks do a bass swap and a crossfader cut to the incoming deck, the \
         outgoing one stepping to its next seed after; one playing net deck steps to its next \
         seed on the line. `style`: filter (a noise riser and a high-pass sweep that end on the \
         line), echo (the room rings out from the line / a delay throw into it), roll (a loop \
         roll 4 -> 2 -> 1 -> 1/2 beat and a riser, dropped on the line), auto (rotates the \
         three). `fade` = bars the effects last (1-8, held under half of `bars`); `effects` false \
         = the bare handover. It arms the same moves as the Perform window (they show in `moves`). \
         Default off; settings are the config keys autodj.enabled / bars / fade / style / effects. \
         action status (default: the settings, state idle | waiting | transition, why it is idle, \
         bars_left to the next handover, the last word, transitions made), on, off, toggle, set \
         {bars, fade, style, effects, on}, next (one transition on the next bar line, even while \
         off), cancel (let go of one in flight).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","on","off","toggle","set","next","cancel"]},"on":{"type":"boolean"},"bars":{"type":"integer","description":"8-64"},"fade":{"type":"integer","description":"1-8"},"style":{"type":"string","enum":["auto","filter","echo","roll"]},"effects":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "setlist",
        tool: Some("djapp_setlist"),
        summary: "set lists: build a whole set in advance (segments of a pair / net seed / saved track, each with a transition, inserts and a tag), generate one from a recipe, save, load, edit",
        default_action: "list",
        actions: A_SETLIST,
        doc: "Set lists (docs/SETLIST.md). A set is an ordered list of segments: a source (a loop-set pair \
              like dnb:82819 group 3, a net deck's seed / spec, or a saved track's CID), its length in bars, \
              and the transition into the next one (a Perform move: bass_swap, filter_exit, echo_out, \
              loop_roll, phrase_sync, vocoder_handoff, or a bare cut), optional Master insert automation \
              (filter_sweep / delay_throw over the last N bars) and an energy / genre tag. Canonical JSON \
              with a content ID (cid), kept as <cid>.json in the app-support folder. One set at a time is \
              the working set (the Set window's editor): list, get, new, generate {segments, genres: \
              \"dnb then jungle\", bars, vary, inserts, seed, name, save} (the same recipe and seed make \
              the same set, from whole A / B pairs only), save, load {id}, delete {id}, duplicate, edit \
              {op: insert | remove | move | bars | transition | inserts | tag | source | rename, index, ...}. \
              Queue it to play with `queue`. Every write answers the working set after it.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "track",
        tool: Some("djapp_track"),
        summary: "saved tracks with a license (CC0-1.0 default | CC-BY-4.0 | all-rights-reserved) and a visibility (public | private): save a deck, relicense, fork, share by CID",
        default_action: "list",
        actions: A_TRACK,
        doc: "Saved tracks (docs/LICENSING.md). A track is what a deck plays (its share payload) kept as one \
              canonical, content-addressed JSON object that carries its own rights: license CC0-1.0 (the default: \
              public domain), CC-BY-4.0 (reuse with credit) or all-rights-reserved; visibility public (the \
              default) or private (never shared by CID, never uploaded); author (a signer identity); forkedFrom \
              (the parent's CID, for a remix) and credits (every CC BY ancestor's CID and author, kept down the \
              whole chain of forks). The rights are inside the object, so changing the license changes the CID. \
              list, get {id | deck}, save {deck, name, license, visibility, author}, license {id, value}, \
              visibility {id, value}, fork {id, name, author} (warns on an all-rights-reserved parent), share \
              {id} (refused when private), load {id, deck}, delete {id}. The music's own share CID (music_cid) \
              never changes with the rights. Generated genre + seed snapshots are CC0 by definition and are not \
              tracks.",
        schema: "",
        params: &[],
        aliases: &["tracks"],
        local: false,
    },
    Noun {
        name: "queue",
        tool: Some("djapp_queue"),
        summary: "the set queue: play sets back to back on the phrase grid, with the transitions and inserts they hold",
        default_action: "status",
        actions: A_QUEUE,
        doc: "The set queue (docs/SETLIST.md): plays set lists back to back on the bar / phrase grid, \
              on Auto-DJ's scheduler. add {id | the working set}, clear, play {from}, pause, stop (an echo \
              out, then the decks stop), next (skip: the transition runs now), jump {n | set + segment}, \
              loop {on}, status (default: state, now and next, bars_left, the queued sets, the last word). \
              The first segment starts on the next 8-bar phrase line (at once when nothing plays); each \
              transition lands on its line within one service pass, and arms the same moves as the Perform \
              window (they show in `moves`). Playing the queue turns Auto-DJ off. `state section=queue` \
              is the same status.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "btw",
        tool: Some("djapp_btw"),
        summary: "btw, the bitwrap language: a text program whose pattern lines are nets on the decks and whose lines script everything djapp does; eval lands on the next bar",
        default_action: "get",
        actions: A_LIVE,
        doc: "btw (docs/BTW.md; `live` and `livecode` are aliases): the bitwrap language. A program is lines: `deck A: kick e(5,8) | \
              snare ~ x ~ x | bass markov(am, 0.7)` (mini-notation: x hit, ~ rest, [..] group, <..> alternation, *n repeat, \
              e(k,n) euclid, notes c3, accents e2! and slides e3^, degrees 1 3 5, generators chord() arp() markov(), \
              transforms transpose() reverse() rotate() every() degrade() slow() fast() octave() humanize()), \
              `every 16 bars: swap pairs with echo_out`, `master: filter lp 2k -> 8k over 8 bars`, `tempo 126`, `key am`; \
              `do <noun> <verb> key=value ...` is any call of the control language, checked against the registry (sugar: \
              `vocoder on`, `freeze phrase bars=2`, `vocal create \"words\" bars=4`, `trick on deck=A`); time words `at bar N`, \
              `on next phrase 16`, `every N bars from M`, `for N bars` (put back), `after`/`before SECTION`, ramps `a -> b over N bars`; \
              `let name = ...`, `def name(a, b) { ... }` called as `name(x, y)`, `repeat N { }`, `section drop 16 bars { }` and \
              `arrange intro build drop`, `import NAME-or-CID`; a song ENDS after its last section (the decks stop on the bar line; \
              `end after outro`, `end at bar 64`, or `loop` to start again); `evolve every 8 bars { melody: rotate 1 | transpose <0 5 7 0>  stop when gen == 12 }` makes generations of a section (each with a CID and the one it came from: `lineage` in eval). A program is total: no recursion, written-out counts. \
              eval {text}: parse, check, compile and stage it; it lands on the next bar (one sent in the last beat of a bar, a bar \
              later: `lands` says which program bar), a deck whose net did not change is not touched, a control line already running \
              keeps its state. A program with an error is refused whole: `ok` false, `diagnostics` with line, col, message, expected, \
              found and a caret `text`, and the last good program plays on. check {text}: the same, staging nothing. get (default): \
              the running program (canonical text, `cid`, `nets`), the section and bar, the staged one, the bar / phrase, errors, \
              warnings (a muted strip), the last words. fmt {text}: the canonical form the CID is of (whitespace, comments, number and \
              note spelling, param order do not change it). save {name, text?, license?, visibility?, author?}: a licensed .btw object \
              (CC0 public unless said); load {id | path, eval?}; list; fork {id}: records forkedFrom and carries credits; license / \
              visibility {id, value}; export {id, path} and import {path | text}: .btw files; share {id | text, sig?, notes?}: a \
              self-sealing link https://beats.bitwrap.io/p#CID.text; open {url}: decodes it, recompiles, recomputes the CID and refuses \
              a mismatch; instruments: every voice, kit and palette `inst=`, `kit=` and `sound=` accept. stop: end the program (the \
              decks play on; a strip a move muted is put back). `#! genre: techno` and `#! tags: acid, drop` header lines link a program to the \
              bitwrap ontology (a misspelt term is a warning in check, with a did-you-mean, and an error in save and import; save {genre, tags} \
              sets them): jsonld {id | text}: the program as JSON-LD; terms {kind}: the vocabulary; generation {gen, scope?}: the CID of generation N of an `evolve`; \
              test {id | text, sample_rate?}: judges the program's `assert` statements (the static ones re-checked, the `level` ones measured on an offline \
              render: it is refused while a program is running or staged, because the render would hold the window and the bar clock; `stop` first, or \
              `djctl btw test FILE`). A program cannot call the verbs that act on the program that runs (`do btw eval`, `stop`, `load`, `open`, `replay`, \
              `perform` are errors at check). The performed tier: a control value or a \
              control line's `when` may name `knob(3)`, `pad(5)`, `fader(A)`, `held(key)` or `held(c4)` (`master: filter lp 200 + knob(3) * 7800`, \
              `every 1 bars: deck B: level 0.5 when held(key)`), read from the controllers at each bar line and logged; \
              perform {do: status | start | stop | save | list} keeps the log as a performance (an object with its own CID), and \
              replay {id} plays it back call for call. The same text is the same CIDs.",
        schema: "",
        params: &[],
        aliases: &["live", "livecode"],
        local: false,
    },
    Noun {
        name: "freeze",
        tool: Some("djapp_freeze"),
        summary: "granular freeze on a strip's history",
        default_action: "status",
        actions: &A_FREEZE,
        doc:
        "Granular freeze on any strip's history: an input (the TD-3, a mic), deck A / B, L (the \
         loop row), V (the vocoder) or M (the master), named by strip (or input). action on \
         (freeze the last `beats` into a grain cloud, stereo for the strips; the source can \
         stop, the cloud carries on), off \
         (thaw), toggle, scrub (by: beats of the frozen audio, negative back; the jog aimed at \
         that history does the same while frozen), set / status. Freezing and thawing land \
         on the next beat while quantize is on (default); now=true does it at once. Knobs: \
         beats (0.25-16), size (grain ms 10-500), density (grains/s 2-200), spread (0-1 of \
         the frozen beats), jitter (± semitones 0-12), position (0-1), mix (0-1); knob+value \
         sets one by its 0-1 travel. phrase (bars): freeze whole bars of an input's beat grid, \
         ending on the next bar line (fired within one UI tick of it: the freeze cannot end \
         on a frame of its own choosing). Logged in the action log.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","set","on","off","toggle","scrub","phrase"]},"bars":{"type":"integer"},"strip":{"type":"string","description":"an input's label, A, B, L, V or M; default the jog-aimed / first input with a history"},"input":{"type":["string","number"],"description":"label or index (strip's older name)"},"now":{"type":"boolean"},"quantize":{"type":"boolean"},"by":{"type":"number"},"beats":{"type":"number"},"size":{"type":"number"},"density":{"type":"number"},"spread":{"type":"number"},"jitter":{"type":"number"},"position":{"type":"number"},"mix":{"type":"number"},"knob":{"type":"string"},"value":{"type":"number"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "vocoder",
        tool: Some("djapp_vocoder"),
        summary: "the channel vocoders: sources, knobs, switches, synth, presets, instances",
        default_action: "status",
        actions: &A_VOCODER,
        doc:
        "The channel vocoders: a modulator shapes a carrier. Up to 4 run at once, each with its \
         own sources, knobs, synth, freeze and strip (V, V2, V3, V4): instance (0-3) names the \
         one a call acts on (default the selected one, the window's tab; naming one not in use \
         adds it); action list | add | remove (instance, not 0) | select (instance) manage \
         them and return vocoders {selected, max, instances}. modulator and carrier each take: \
         none | synth (the internal synth: set notes, e.g. [\"A2\",\"E3\",\"A3\"] or MIDI \
         numbers; note+down plays one key) | input:LABEL (a live strip, e.g. a mic or the \
         TD-3; name it) | deck:A | deck:B (a deck, live and pre-fader: whatever is loaded on \
         it) | clip:ID | slot:N (a loop-row clip) | library:ID | capture:LABEL:BEATS (the last \
         beats of an input's history); loops follow the master tempo. A deck or the synth is \
         on one side only. swap=true (or action swap) exchanges the two: a drum loop shaping \
         the synth (talking drums) becomes the synth shaping the drums. \
         Setting the carrier never changes the modulator, and the reverse. \
         Sources are tapped pre-fader, so a muted mic still drives it. Knobs in their own \
         units: bands (4-32), shift (formant shift, semitones ±12), attack / release (ms), mix \
         (dry/wet 0-1), noise (unvoiced blend 0-1), volume (strip V's fader), vowel (0-4 or \
         a/e/i/o/u) and vowel_mix (the formant filter, 0 off), synth_level, synth_detune \
         (cents: the spread between osc 1 and 2). The synth carrier is a 3-oscillator voice: \
         osc1_/osc2_/osc3_ wave (saw|square|triangle|sine|noise or 0-4), _oct (-3..3), _fine \
         (cents ±100), _level (0-1); pwm (0.05-0.5), sync (on/off: osc 2 hard-synced to osc \
         1), ring (0-1), cutoff (Hz 30-18000), resonance (0-1), env_amount (-1..1), env_decay \
         (ms), synth_attack / synth_release (ms), drift (0-1). preset: NAME loads a synth \
         preset (classic, acid, pad, sync lead, ring bell, noise sweep, or one saved). \
         Switches: on, insert (replace the carrier's own strip signal instead of \
         playing on V), mute, pfl, bypass (the carrier straight onto V without the modulator: \
         the synth as a jam synth), freeze (hold the band envelopes: the carrier keeps the \
         shape with the modulator gone), xfade (the crossfader morphs V: carrier left, vocoded \
         centre, modulator right; the decks leave the crossfader, mix rests). knob+value sets a knob by its 0-1 travel as a binding does. \
         action: status (default) | set | cycle_modulator | cycle_carrier (dir ±1) | swap | \
         preset_save (name) | preset_delete (name) | preset_next | preset_prev. voice: NAME \
         (a formant voice preset, docs/VOICE.md) | off makes the internal synth sing (its notes, \
         vowel knob and level), and a source spelled voice or voice:NAME is that synth: a mic \
         talking through a robot choir. Returns \
         the vocoder (instance, label): settings, sources, notes, levels, routing, frozen, each band's hz, \
         carrier_hz, env, peak and db. Logged \
         in the action log.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","set","cycle_modulator","cycle_carrier","swap","preset_save","preset_delete","preset_next","preset_prev","list","add","remove","select"]},"instance":{"type":"integer","minimum":0,"maximum":3},"modulator":{"type":"string"},"carrier":{"type":"string"},"beats":{"type":"number"},"on":{"type":"boolean"},"insert":{"type":"boolean"},"mute":{"type":"boolean"},"pfl":{"type":"boolean"},"bands":{"type":"integer","minimum":4,"maximum":32},"shift":{"type":"number"},"attack":{"type":"number"},"release":{"type":"number"},"mix":{"type":"number"},"noise":{"type":"number"},"volume":{"type":"number"},"vowel":{"type":["number","string"]},"vowel_mix":{"type":"number"},"synth_level":{"type":"number"},"synth_detune":{"type":"number"},"osc1_wave":{"type":["string","number"]},"osc1_oct":{"type":"integer"},"osc1_fine":{"type":"number"},"osc1_level":{"type":"number"},"osc2_wave":{"type":["string","number"]},"osc2_oct":{"type":"integer"},"osc2_fine":{"type":"number"},"osc2_level":{"type":"number"},"osc3_wave":{"type":["string","number"]},"osc3_oct":{"type":"integer"},"osc3_fine":{"type":"number"},"osc3_level":{"type":"number"},"pwm":{"type":"number"},"sync":{"type":["boolean","string","number"]},"ring":{"type":"number"},"cutoff":{"type":"number"},"resonance":{"type":"number"},"env_amount":{"type":"number"},"env_decay":{"type":"number"},"synth_attack":{"type":"number"},"synth_release":{"type":"number"},"drift":{"type":"number"},"bypass":{"type":"boolean"},"freeze":{"type":"boolean"},"xfade":{"type":"boolean"},"swap":{"type":"boolean"},"preset":{"type":"string"},"voice":{"type":"string","description":"the synth sings: a formant voice preset (robot, robot-choir, choir-aah, …) or off; a source may also be spelled voice or voice:NAME (docs/VOICE.md)"},"name":{"type":"string"},"notes":{"type":["array","string","integer"],"items":{"type":["integer","string"]},"description":"MIDI numbers or names: [45,\"C3\"], or \"45,48,52\" / \"A2,C3,E3\"; a bad note is an error"},"note":{"type":["integer","string"]},"down":{"type":"boolean"},"knob":{"type":"string"},"value":{"type":"number"},"dir":{"type":"integer"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "voice",
        tool: Some("djapp_voice"),
        summary: "the formant voice: robot / choir vocals sung by the nets (presets, vowels, a sung part, the vocoder's voice)",
        default_action: "status",
        actions: A_VOICE,
        doc: "The formant voice (docs/VOICE.md): stylised robot and choir vocals synthesised with \
              no recorded singing. A source (saw, pulse or glottal pulse) through five formant \
              resonators sings a vowel (a e i o u, and the choir's ah / oo) or a syllable (da, ro, \
              moo: one consonant then a vowel), morphing from note to note, with vibrato and a \
              unison choir. The nets sequence it like notes: a transition's midi binding carries \
              \"vowel\": \"da\", and the generator's vox option writes a choir or robot part (net \
              vox, channel 8). The voices are the instruments voice-<preset>. status: presets, \
              vowels, onsets and each deck's sung nets. presets: the list. vowel (deck?, vowel, \
              net?): the vowel a deck's voices sing, live (notes with their own syllable keep \
              it). load (deck?, genre?, seed?, style?, preset?, structure?): compose a track with a \
              sung part onto a deck. vocoder (preset | off, carrier?): the vocoder's internal \
              synth sings with that voice (its notes, its vowel knob), so a mic talks through a \
              robot choir. Not vox: that is the vocal sample pads.",
        schema: "",
        params: &[],
        aliases: &["formant"],
        local: false,
    },
    Noun {
        name: "vocal",
        tool: Some("djapp_vocal"),
        summary: "vocals from typed lyrics: words sung on pitch by the formant voice, CC0 by default (a line you can save, fork, license and send to the loop row, a deck or the vocoder)",
        default_action: "status",
        actions: A_VOCAL,
        doc: "Typed lyrics, sung (docs/VOICE.md): lyrics go through the BeatScript lexicon (CC0; \
              letter-to-sound rules written here for unknown words; no espeak-ng), or are written in \
              BeatScript phonemes between slashes (`/w ee - aa r/`, docs/btw/phonemes.md), \
              into phonemes and syllables, are fitted to a key and a bar grid, and are sung on pitch by \
              the robot / choir formant voice: robotic by design, with the consonants f v z sh zh ch j th \
              dh ng, codas and diphthongs. A line is canonical JSON with a CID, its own license (CC0 by \
              default), visibility, author and lineage, so it saves, forks (a fork credits its parent) \
              and shares like a track. Example: create lyrics=\"I P O, Claude is on the road\" key=\"Bb \
              minor\" bpm=124 bars=2, then to_row play=true (it lands on the loop row, exactly two bars \
              long, so it loops on the bar), or to_vocoder (the words as the modulator), save, render \
              path=hook.wav. create fits at most sixteen syllables a bar; notes=… gives your own notes. \
              words shows how a text will sound without making anything. hook (deck?, lyrics): add a \
              vocal hook in a playing track's key and tempo. play is the headphone cue only.",
        schema: "",
        params: &[],
        aliases: &["lyrics"],
        local: false,
    },
    Noun {
        name: "render",
        tool: Some("djapp_render"),
        summary: "offline render of an arrangement or the action log (runs in the MCP server, not the socket)",
        default_action: "",
        actions: &[],
        doc:
        "Offline render (docs/HEADLESS.md), faster than real time and the same every time, \
         without touching what the app plays: an arrangement (inline as arrangement, or a file \
         path; a session JSON or a journal file works too) or the app's own action log \
         (from_log=true, optionally from_bar/to_bar). Renders on the engine djapp plays: both \
         decks, the loop row and library clips, sample decks, the vocoder (any modulator and \
         carrier, the synth), the granular freeze, the tricks, the FX units and master chain. \
         Writes out (default a temp WAV); stems=DIR also writes one WAV per strip (A, B, loops, \
         each input, vocoder); inputs={LABEL: file} feeds an input strip from audio. Returns \
         the files, seconds, peak/RMS dBFS, render time and any moves the engine refused.",
        schema: r#"{"type":"object","properties":{"arrangement":{"type":"object","description":"a djrust.arrangement/1 document"},"path":{"type":"string","description":"an arrangement, session or journal file"},"from_log":{"type":"boolean","description":"render the running app's action log"},"from_bar":{"type":"integer"},"to_bar":{"type":"integer"},"out":{"type":"string","description":"WAV to write"},"stems":{"type":"string","description":"directory for one WAV per strip"},"inputs":{"type":"object","description":"input label -> audio file fed into it","additionalProperties":{"type":"string"}},"beats":{"type":"number","description":"render exactly this many beats"},"tail_beats":{"type":"number","description":"beats after the last move (default 4)"},"sample_rate":{"type":"number","description":"default 48000"},"loops_dir":{"type":"string","description":"capture library, over the arrangement's"},"float32":{"type":"boolean"},"post":{"type":"string","description":"finish the master with a post preset (docs/POST.md): none, clean-medium (default for true), clean-light, clean-strong, expander, treble-shelf, master; stems stay raw","enum":["none","clean-medium","clean-light","clean-strong","expander","treble-shelf","master"]},"post_chain":{"description":"a custom post chain (array of stages / preset names), over post"}}}"#,
        params: &[],
        aliases: &[],
        local: true,
    },
    Noun {
        name: "post",
        tool: Some("djapp_post"),
        summary: "the post chain: live on the master, and offline on files",
        default_action: "status",
        actions: &A_POST,
        doc:
        "The post-processing (master) chain, docs/POST.md. On files, offline and natively \
         (no ffmpeg; never touches what plays): action process (in, out .wav or .mp3, preset \
         or post_chain), measure (in), presets (the list with each one's stages and the \
         ffmpeg graph it stands for). Returns before / after LUFS, LRA, true peak and the \
         noise-floor estimate. Live, on djapp's master before the master fader (off by \
         default; the master is unchanged until it is turned on): action status (default) \
         or set (on, toggle, preset, chain, step ±1); the loudness stage runs live as a \
         short-term-LUFS leveler + true-peak limiter, a denoise adds ~23 ms of latency. Live \
         changes are logged in the action log and journal, so renders of the log replay \
         them. Turn it on or change the preset only when asked: the booth hears it.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","set","process","measure","presets"]},"in":{"type":"string"},"out":{"type":"string"},"preset":{"type":"string","enum":["none","clean-medium","clean-light","clean-strong","expander","treble-shelf","master"]},"post_chain":{"description":"files: a custom chain (array of stages / preset names)"},"chain":{"description":"live: a custom chain"},"on":{"type":"boolean"},"toggle":{"type":"boolean"},"step":{"type":"integer","enum":[-1,1]},"float32":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "heos",
        tool: Some("djapp_heos"),
        summary: "the Denon HEOS zones: volume, mute, transport, groups, the master streamed to them",
        default_action: "status",
        actions: &A_HEOS,
        doc:
        "The Denon HEOS zones on the LAN (found by SSDP + mDNS; no addresses configured): each \
         zone's volume (0-100), mute, play state and now playing, and their groups. action: \
         status (default: zones, groups, the stream, the CLI log) | volume (zone, level 0-100 or \
         step ±1-10; max caps level) | mute (zone, on true/false, omitted toggles) | play / pause \
         / stop (zone) | group (zones, the first leads) | ungroup (zone or group) | stream (zones, \
         format flac (default) or wav: serves djapp's master over HTTP and has the zones play \
         it, grouping several first; it plays several seconds (2-5 s) behind the booth, so it is \
         for the room, never for cueing: headphones stay the monitor) | stream_stop (zones, \
         default the ones streamed to) | stream_server_stop | select (zones; on false unticks, \
         true adds, omitted: exactly these: what zone=selected and the window's buttons use) | \
         discover | connect (host, or auto) | refresh | open (the window). zone / zones: a \
         name or part of one (Kitchen, pat), a group's name, a pid, selected or all; a list or \
         comma-separated. Commands are sent at once and answered by the zones a moment later: \
         read status again to see the result. These are real speakers: change volume in small \
         steps unless asked. Logged in the action log.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","volume","mute","play","pause","stop","group","ungroup","stream","stream_stop","stream_server_stop","select","discover","connect","refresh","open"]},"zone":{"type":["string","number"]},"zones":{"type":"array","items":{"type":["string","number"]}},"level":{"type":"number","minimum":0,"maximum":100},"step":{"type":"integer","minimum":-10,"maximum":10},"max":{"type":"number","minimum":0,"maximum":100},"on":{"type":"boolean"},"format":{"type":"string","enum":["flac","wav"]},"host":{"type":"string"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "config",
        tool: Some("djapp_config"),
        summary: "the config file: get / set / diff / save / reload / keys / path",
        default_action: "get",
        actions: &A_CONFIG,
        doc:
        "djapp's config file (~/Library/Application Support/dj-vevr-se/djapp.json): the settings a \
         restart comes back with (output and input devices, input channels and latency, the \
         input history, the TD-3 / PD705 clock and keys, the controller filter, each deck's \
         palette / Auto-next / pitch range, master and headphone levels, which windows are open). \
         Edits to the file apply live and changes in the app are written back. action: get \
         (key: a key or a section like td3; none: all), set (values: {key: value} or key + value; \
         applied live, saved; keys that need a restart are saved and listed under \
         restart_needed), diff (against: defaults | file | launch), save (every key as it runs \
         now), reload (re-read the file, apply every key), keys (the table: type, default, live, \
         flag), path. Key names: action=keys.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["get","set","diff","save","reload","keys","path"]},"key":{"type":"string","description":"a key (td3.offset_ms, audio.input_latency_ms.TD-3) or a section (td3, panels)"},"value":{"description":"set: the value for key"},"values":{"type":"object","description":"set: {key: value, ...}"},"against":{"type":"string","enum":["defaults","file","launch"]}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "perf",
        tool: Some("djapp_perf"),
        summary: "the perf overlay and its numbers",
        default_action: "status",
        actions: &A_PERF,
        doc:
        "The window's perf overlay and its numbers: fps (frames drawn in the last second), the \
         display's rate (estimated from the shortest frame interval, or set), frame interval \
         p50/p95/max and the work inside a frame (ms), late frames (> 1.5 display periods) and \
         the frames they dropped, whether the window draws continuously; the audio output's \
         buffer, longest callback gap and underruns (xruns + silent fills + late callbacks); \
         per deck while it plays, how often the published playhead moves and on what share of \
         frames the raw value and the drawn platter stood still. action: status (default) | on \
         | off | toggle (the overlay; F3 in the window) | reset (count from now) | set \
         (target_hz: the display's rate, 0 = estimate).",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","on","off","toggle","reset","set"]},"target_hz":{"type":"number"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "browse",
        tool: Some("djapp_browse"),
        summary: "the loop / sample browser: search, load, audition",
        default_action: "status",
        actions: &A_BROWSE,
        doc:
        "The loop / sample browser: the capture library, recent files, ~/Music and the system's \
         Apple Loops (GarageBand's packs, read in place, never copied). status (default): \
         counts per source, scan, the audition. packs (source?): the pack folders. list / \
         search: query (words matched against name, pack, category, genre, key), kind \
         (vocals, drums, bass, synth, fx), source (captures, recent, music, apple-loops), pack, \
         offset, limit (default 50); each item has path, pack, category, tempo, key, beats, \
         seconds and its index. load (path, or index with the same query; to: A | B | row \
         (onto the loop row, waiting for its pad) | row_play (looping from the next launch) | \
         vocoder (the modulator)). audition (path or index): play it on the headphone cue only, \
         looping (again on the same file stops it); stop; gain (value 0-1). open (on), rescan, \
         select (query + index: the window's highlight, what the Inpulse's LOAD takes). Files \
         decode in the background: load returns at once and status.pending shows what is still \
         on its way.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","sources","packs","list","search","load","audition","stop","gain","open","rescan","select"]},"query":{"type":"string"},"kind":{"type":"string","enum":["vocals","drums","bass","synth","fx"]},"source":{"type":"string","enum":["captures","recent","music","apple-loops"]},"pack":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"},"path":{"type":"string"},"index":{"type":"integer"},"to":{"type":"string","enum":["A","B","row","row_play","vocoder"]},"value":{"type":"number"},"on":{"type":"boolean"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "ping",
        tool: None,
        summary: "is it alive (answered on the socket's thread)",
        default_action: "",
        actions: &[],
        doc: "Is djapp alive: answers ok, app and pid without waiting for the window.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "methods",
        tool: None,
        summary: "the method names (the nouns); describe has the verbs",
        default_action: "",
        actions: &[],
        doc: "The socket's method names. describe lists them with their verbs and params.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "describe",
        tool: Some("djapp_describe"),
        summary: "the control language itself: every noun, verb and param, from the registry",
        default_action: "",
        actions: &[],
        doc: "The control language, from the registry the tool list and the docs are made from. \
              No params: the language in a paragraph and every noun with its verbs. noun: that \
              noun's doc, verbs (with their effect: read, write, or live = reaches the speakers, \
              gear, network or disk) and params. noun + action: one verb. search: the nouns and \
              verbs whose name or doc has the word. Answered at once, even while the window is \
              busy. `help` is the same.",
        schema: r#"{"type":"object","properties":{"noun":{"type":"string","description":"a noun (the method / tool name without djapp_)"},"action":{"type":"string","description":"with noun: one verb"},"search":{"type":"string","description":"find nouns and verbs by word"}}}"#,
        params: &[],
        aliases: &["help"],
        local: false,
    },
    Noun {
        name: "net",
        tool: Some("djapp_net"),
        summary: "djnet, the LAN protocol: pairing, peers and their clocks, tracks by CID, the phone's stream",
        default_action: "status",
        actions: A_NET,
        doc: "djnet (docs/NET-PROTOCOL.md; builds with the `network` feature, otherwise it says so): \
              status (the host, port, pair_code, trust, discovery, peers with clock offset, round \
              trip, BPM and phase against the master, the paired list, the log), track (deck, cid, \
              payload, play, sync: a beats track by CID), glance, stream (url: the phone's \
              /master.djl into the Phone strip) / stream_stop / stream_status, aux_sync / aux_lock \
              (on) / aux_nudge (beats) / aux_trim (ms) / aux_latency (ms) / aux_match, code (a new \
              pairing code), trust (code | open | off), forget (id | all), open.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "loopsets",
        tool: Some("djapp_loopsets"),
        summary: "the Loops window: loop collections (Acid Bass, a genre's set, genre:seed), their A / B groups on the decks",
        default_action: "status",
        actions: A_LOOPSETS,
        doc: "The Loops window (docs/ANDROID-PARITY.md, \"Loop collections\"), the same calls as its \
              buttons. A collection is named by a spec: acid-bass, a genre's curated set (techno, \
              dnb, house, ...), or a set generated from a seed (dnb:2602: the same on every device, \
              CID too). Its loops come in numbered groups of an A and a B (1A 1B, 2A 2B, ...). \
              status: the collection picked, the held loops, the lit pair. collections: the picker \
              (curated and kept). groups: the picked set's loops. pick (spec): show a collection. \
              new_set (genre?, seed?): the New set button. keep (spec?, on?): the star. drums (on): \
              the Drums checkbox. load (deck, id | group + side, spec?): one pad: that loop on that \
              deck, playing, locked on the other deck's bar. load_pair (group, spec?): the A + B \
              pad, group N's A on deck A and its B on deck B. open (on): the window. Naming spec on \
              load / load_pair picks that collection first, so `loopsets load_pair spec=dnb:2602 \
              group=2` is the whole job. Not the loop row: that is `loops` / `clips`.",
        schema: "",
        params: &[],
        aliases: &["collections", "loop_collections"],
        local: false,
    },
    Noun {
        name: "windows",
        tool: Some("djapp_windows"),
        summary: "the windows: which are open, and open / close / toggle any by name (the top bar's buttons)",
        default_action: "list",
        actions: A_WINDOWS,
        doc: "The windows, as the top bar's buttons: list, open / close / toggle (window: td3, mpk, \
              kits, bindings, log, wheel, loops, perform, effects, inserts, aux, net, heos, menu, \
              map, midi, route, vocoder, browser, perf, set), closeall. A window has to be open to be \
              seen in djapp_screenshot; the verbs of what it holds work with it shut.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "inserts",
        tool: Some("djapp_inserts"),
        summary: "the Inserts window: filter / crush / delay / reverb on A, B, the Wheel or the Master",
        default_action: "status",
        actions: A_INSERTS,
        doc: "The Inserts window (docs/ANDROID-PARITY.md): filter -> crush -> delay -> reverb on \
              target A, B, Wheel (its input strip) or Master, each switched and turned live; the \
              delay follows the deck's tempo (the Wheel's and the Master's: the lead deck's). status \
              (target?): the chains with each knob's value and display. on (target?, effect, on?), \
              set (target?, param e.g. delay.mix, value 0..1), off (ALL OFF). The knobs are kept in \
              inserts.json like the window's. The audible ones change what the booth hears.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "effects",
        tool: Some("djapp_effects"),
        summary: "the Effects window: every trick and macro pad on A, B, the Wheel or the Master",
        default_action: "status",
        actions: A_EFFECTS,
        doc: "The Effects window (docs/ANDROID-PARITY.md): the engine's catalogue of tricks and \
              macros as pads, fired on the target's next bar (the Wheel's and the Master's: the \
              lead deck's) for 1-4 bars or latched. status (target?): the aim, length and what is \
              on. catalogue: the pads. tap (id): a click: off on the next bar if on, else on. hold / \
              release (id): a transition pad held. length (1-4 | latch), sweep (bars), knob (the \
              filter, 0.5 open), panic, target (aim the window). Pads that work a deck's track \
              are refused on the Wheel and the Master, with the reason.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "wheel",
        tool: Some("djapp_wheel"),
        summary: "the Wheel window: windwheel's tonewheel instrument (play, instrument, motor, key, level)",
        default_action: "status",
        actions: A_WHEEL,
        doc: "The Wheel window (windwheel's linear tonewheel on the Windwheel input strip): magnets \
              on a cord, a drop or a pluck through a hoop wound with a patterned coil. status: the \
              instrument and its motion and sound, play state and meters (busy: a worker is still \
              winding a change). presets, load (preset), play / pause / toggle, motor (on, speed \
              m/s), interval (seconds, or beats of a deck's tempo: gravity drops on the grid), sync (deck, ratio: the motor so the wheel's tempo is a whole multiple or divisor of the deck's), step (deck, up: one rung), follow (deck), pluck (tube?), level (db), trigger (on, \
              pull), key (root, transpose), brightness, sequence (notes). The strip's mixer knobs \
              are djapp_input input=Windwheel. Not yet here: a hand on the drawing, the build knobs (magnets, carrier, load, timbre, decay).",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "controllers",
        tool: Some("djapp_controllers"),
        summary: "controllers and their Mixxx mappings: what drives each port, load / unload a mapping, what it left unmapped",
        default_action: "status",
        actions: A_CONTROLLERS,
        doc: "MIDI controllers beyond the hand-made profiles (Inpulse 200 MK3, APC mini mk2, MPK mini \
              mk2, TD-3, PD705): a controller Mixxx has a mapping for works when plugged in, from the \
              user's own Mixxx install or a folder named by the config's mixxx.dir (docs/MIXXX-MAPPINGS.md; \
              nothing is bundled or downloaded: the mappings are GPL). status: the folders, the count \
              of mappings, each input port with what drives it. list (filter): the mappings found. \
              load (port, mapping) / unload (port or device): put a mapping on a port or take it off; \
              answers the counts and the notes. report (device or port): every control that mapped, \
              with its target, and every one that did not, with the reason (needs the script, nothing \
              in djrust for it, ...). rescan: read the folders again. map (port, mapping | none): the \
              config override, saved. XML mappings only: a control with a script binding is listed as \
              unmapped. A hand-made profile always keeps its port.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "menu",
        tool: Some("djapp_menu"),
        summary: "the ≡ Menu window: keep the computer awake, the Toy Box sound pack, About",
        default_action: "status",
        actions: A_MENU,
        doc: "The ≡ Menu window. status: keep_awake, whether it is holding the computer awake now, \
              the Toy Box pack (installed or not, the SoundFont folder), version, sample rate, \
              features. keep_awake (on): the Settings checkbox. toybox: the Install / Update \
              button (writes djrust's own synthesized SoundFont into the folder; needs the sf2 \
              build; ask first).",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "batch",
        tool: Some("djapp_batch"),
        summary: "several commands as one request: all or nothing, or landing together on the bar grid",
        default_action: "status",
        actions: A_BATCH,
        doc: "Batches (docs/CONTROL.md, \"Batches\"): run an ordered list of commands as one request. \
              run {commands: [{method, params, delay?}, ...]}, each command in any form the language \
              takes ({\"method\":\"mixer\",\"params\":{\"deck\":\"A\",\"mute\":true}}, params beside the \
              method, or \"mixer.mute\"). Options: atomic (every command is checked against the registry \
              first and none runs if any is invalid; then all run in one service pass of the window; \
              no delays), stop_on_error (default true, false when atomic), at (now | next_beat | \
              next_bar | next_phrase | bar:N: the batch waits for that line of the grid deck and lands \
              with the Perform moves; with no deck playing it lands at once), deck (which deck's grid), \
              dry_run (validate and report what each command would change, run nothing). A command's \
              `delay` (beats, or \"1 bar\"; at most 16 beats) staggers a short sequence after the \
              landing, e.g. a filter sweep in steps: anything longer is a set list or the live-code \
              language. Answers the batch: id, state (pending, running, done, cancelled), and per \
              command ok / error with the noun's state answer. A batch that lands now with no delays \
              is done in the answer; a timed one answers pending, and status {id} reads its results. \
              cancel {id} stops what has not run. A batch cannot hold a batch, or render (MCP only), or \
              post's file actions. Sending a JSON array of requests on the socket is the same as a \
              batch with stop_on_error off, answered as an array of {id, result | error}.",
        schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["status","run","cancel"],"description":"default status; run when commands is given"},"commands":{"type":"array","description":"an ordered list of commands, each in any form the language takes","items":{"type":"object","properties":{"method":{"type":"string","description":"a noun, or noun.verb"},"params":{"type":"object","description":"the noun's params (action is the verb); they may also sit beside method"},"delay":{"type":["number","string"],"description":"beats (or \"1 bar\") after the landing, at most 16 beats; not with atomic"}},"required":["method"]}},"atomic":{"type":"boolean","description":"validate every command first and run none if any is invalid; then all in one pass"},"stop_on_error":{"type":"boolean","description":"default true, false when atomic"},"at":{"type":"string","description":"now (default) | next_beat | next_bar | next_phrase | bar:N"},"deck":{"type":"string","description":"the grid deck to time on (A or B)"},"label":{"type":"string"},"dry_run":{"type":"boolean","description":"validate and report what would change; run nothing"},"id":{"type":"string","description":"status / cancel: the batch id (b1)"}}}"#,
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "key",
        tool: Some("djapp_key"),
        summary: "musical keys: each deck's key in Camelot, how the decks mix, key detection, key lock",
        default_action: "status",
        actions: A_KEY,
        doc: "Keys for harmonic mixing (docs/KEY.md). A generated track's key is the one it was composed in \
              (its project's root and scale, as loop sets write it: \"A minor\"); a sample deck's is estimated \
              from its audio (a 12-bin chroma against the Krumhansl-Kessler, Temperley and Albrecht-Shanahan \
              key profiles, averaged), or hand-set. Keys are shown in Camelot (8A = A minor, 8B = C major): \
              same number or one step round the wheel mixes; status answers each deck's key, the compatibility \
              (same, relative, adjacent, diagonal, boost, parallel, clash) with a hint, and when they clash the \
              smallest transpose of B that mixes. Key lock: a sample deck's tempo change through `key tempo` \
              keeps its pitch (a time-stretched copy, WSOLA, swapped in at the same place); net decks are \
              always locked (their notes are synthesised at the tempo asked).",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "analysis",
        tool: Some("djapp_analysis"),
        summary: "beat, tempo and bar-1 detection for audio djrust did not make (Beat This! or the DSP detector)",
        default_action: "status",
        actions: A_ANALYSIS,
        doc: "Beat and downbeat detection (docs/BEAT-DETECTION.md). `beats` finds the tempo, the \
              beats and bar 1 of a capture or loop (target capture|clip, id), an input's history \
              (target input, input), a file (target file, path), a library loop (target library, id) \
              or a folder of vocals (target vox, path): the DSP grid at once, then Beat This! \
              (about 11 MB of models fetched on first use, checksum-verified; without them the DSP \
              grid stands) fitted so bar 1 is a downbeat. It runs off the audio thread and is cached \
              by the audio's content hash; the call answers with the state and any grid already \
              known, `grid` reads it later. New captures are trimmed to start on the downbeat and run \
              whole bars (capture_trim). `set` changes neural, capture_trim, min_confidence, snap \
              (off|beat|bar: a released history scrub lands on that line of the input's grid), \
              history_auto, history_seconds. status shows the worker, the models and every grid. \
              models / fetch_models / retry are the model files. Latency calibration is \
              `input calibrate_latency`.",
        schema: "",
        params: &[],
        aliases: &[],
        local: false,
    },
    Noun {
        name: "mashup",
        tool: Some("djapp_mashup"),
        summary: "live mashups from stems: drums from one track, bass from another, beat-synced and key-aware, content-addressed with parent CIDs",
        default_action: "status",
        actions: A_MASHUP,
        doc: "Mashups (docs/MASHUPS.md). Add source tracks (genre:seed, deck:A, audio:<cid>), assign a stem of \
              each to a slot (drums from A, bass from B, melody from C), set each part's level, mute, solo and \
              transpose. Everything plays at the mashup's tempo; tonal parts are checked against the mashup's \
              key and clashes are warned with the transpose that fixes them (key auto=true applies it). A \
              generated track's stems are its nets, so the mashup is realised as one raw-nets share a deck plays \
              (play). External audio needs a stem separator: the Demucs source is a stub until demucs-rs. \
              A mashup is canonical JSON with a CID; its parents are every source's CID and a fork points back \
              (forkedFrom). Saved as <cid>.json in the app-support folder.",
        schema: "",
        params: &[],
        aliases: &["mashups"],
        local: false,
    },
    Noun {
        name: "record",
        tool: Some("djapp_record"),
        summary: "set recording: the master to a WAV plus a timestamped CID tracklist and an exact replay, signed and content-addressed",
        default_action: "status",
        actions: A_RECORD,
        doc: "Set recording (docs/RECORDING.md). start records the master output (after the master fader) to a \
              32-bit float WAV on a worker thread, and the action log from that bar. stop builds the recorded \
              set: the audio's own CID, a tracklist of every track, mashup, loop set and transition with its CID \
              and time (derived from the action log, plus marks), the arrangement that replays it, and the \
              parent CIDs; canonical JSON with a CID, signed by the booth's key (EIP-191 over the CID, signer a \
              did:pkh), saved under <support>/recordings/<cid>/. replay plays it again exactly through the action \
              log's player; fork makes an editable copy that points back; verify checks the CID, the audio and \
              the signature.",
        schema: "",
        params: &[],
        aliases: &["recording", "setrec"],
        local: false,
    },
];
