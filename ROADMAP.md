# Corydoras roadmap: Phase 3

Starting point: version 0.6.0. Corydoras turns mood boards into versioned design languages, and agents read them through the MCP server together with hard rule sets.

**Phase 3 theme: close the loop.** Before Phase 3 the app fed agents but never learned what they made. Phase 3 links agent output back to the language, checks it against the rules, and uses the result to improve the language.

## Order of work

| Milestone | Theme | Items | Depends on | Status |
| --- | --- | --- | --- | --- |
| 3.1 | Outputs and conformance | 1, 2 | none | Done, in 0.7.0 |
| 3.2 | Geometry and print feedback | 3, 4, 10 | 3.1 | Done |
| 3.3 | Better languages | 5, 6, 7, 8 | 5 is independent; 6 and 7 benefit from 3.1 | Done |
| 3.4 | Plumbing and workflow | 9, 11, 12, 13 | 13 needs 3.1 | Done |

All 13 items are built. The README describes how each one works; this file keeps what was decided on the way.

## 3.1 Outputs and conformance

**1. Generated outputs as first-class items**
- Attach a render or model to a language version. Store it in the library next to `items/`.
- Mark each output accepted or rejected, with an optional note.
- A rejected output becomes an anti-reference. Pass 2 reads accepted outputs as supporting evidence and rejected ones as "do not" evidence.
- Done when: an output can be added from an MCP tool and decided in the app, and a rejection changes the next synthesis.
- Decided: images only for now. Outputs are sent with the MCP tool `submit_output`; there is no add button in the app yet.

**2. Conformance-check MCP tool (`check_against_language`)**
- An agent submits measurements and/or a screenshot for a family.
- The tool returns each applicable rule as pass, fail or unknown, with the evidence IDs behind it.
- Hard rules are checked first and win over language rules.
- Read-only, like the other MCP tools, except that results can be saved as an output record.
- Done when: an agent can check a finished model and get a per-rule verdict.
- Decided: the check uses submitted numbers only, not the vision model. A name or unit that matches nothing is UNKNOWN, never a pass. Without a family, only hard rules can judge.

## 3.2 Geometry and print feedback

**3. Geometry validation against hard rules**
- Check an STL or STEP against the active rule set: minimum wall, overhang, clearance.
- Reuse ScanRuler's thickness and clearance measurements where possible.
- Exposed as an MCP tool and as a button on an output.
- Done: the MCP tool `check_geometry`, and **Check a model…** on an output. The numbers go through the same check as `check_against_language`.
- Decided: Corydoras measures the file itself, so the check works without another app. STL, 3MF and OBJ are read directly. STEP goes through FreeCAD's console program, because the Rust STEP readers that were tried stopped on real CAD exports; without FreeCAD, a STEP file gets a message that says so.
- Decided: a wall is the material between two faces that are near-parallel, so a chamfer or a taper is not reported as a thin wall. Overhangs are judged against the build direction (default +z); the first two layers do not count, and a flat ceiling is measured as a bridge when material holds both ends. Clearance is the smallest open gap between two bodies; faces that touch count as contact, not as a gap of zero.
- Decided: a file with more than one body gets no overhang verdict unless the build direction is given, because an assembly is not in print orientation.
- Decided: ScanRuler is not called. Its numbers (or those of any other tool) can be sent with the file in `measurements`; a sent number replaces the computed one, and the report says so.

**4. Print-feedback log**
- Record a print result (failed, frosted, warped, fine) against a rule or material.
- Feed the log into rule sets and into the conformance check.
- Seed it with the known finding that translucent PETG prints frosted.
- Done: **Print feedback** on each rule set, **Log a print…** on an output, and the MCP tools `log_print_result` and `get_print_log`.
- Decided: the log never changes a rule. The rule set shows a hint when prints disagree with a limit ("a print failed at 1.3 mm, which the limit of 1.2 mm allows"), and you edit the rule. In the check, a number that passes a rule but is at or past a value that failed becomes WARN.
- Decided: agents can add entries, because the designer often tells an agent how a print came out. Only the app removes entries. An entry without a rule set applies everywhere.

**10. Slicer profile import**
- Create a rule set from a Bambu Studio or Orca profile.
- Show where the imported values differ from the built-in "FDM · PETG · Bambu P1S" set.
- Done: the import button next to **Rule sets**. It lists the installed profiles of both slicers and also reads a profile file or a project file (`.3mf`).
- Decided: a profile says how the printer lays lines, not what a designer may draw, so each rule is worked out from the profile and says how (minimum wall = two outer lines, embossed detail = three layers, maximum overhang = 90° minus the support threshold). Values that a profile cannot give (clearances, hole oversize, the instructions) can be copied from the built-in set.

## 3.3 Better languages

**5. Version diff, rollback and pinning**
- Show a semantic diff between versions (for example "blade pitch 4.0 → 3.5, evidence added").
- Let a project roll back to or pin an earlier version.
- Done: **Changes** in the Versions tab, **Pin in…** and **Restore** on each version, and the MCP tool `compare_versions`.
- Decided: Restore saves the old version again as the newest one, so the history stays complete and nothing is deleted. A project keeps its pin; Restore is for agents that read the language without a project.

**6. Coverage report**
- List what a language cannot answer: families with one reference, missing views, contradictions between items.
- Suggest what to add.
- Done: the **Insight** tab and the MCP tool `get_coverage`.
- Decided: the report is worked out from the latest version and the analyses, with no model call, so it is free and always current. Pass 1 now also names the view of each image (three-quarter, front, side, top, back, detail), which the missing-views check uses; for an image that was analyzed before, the words of its description decide.

**7. Language inheritance and merge**
- A language can extend a base and override a few rules.
- Define how the MCP server resolves an inherited rule.
- Done: **Extends** in the language's Edit form, and the inheritance panel in the Document tab.
- Decided: the merge happens when a version is written, not when an agent reads. A language with a base has no synthesis of its own: each of its versions is the base document with the overrides applied, saved as a complete document that names the base, the base version and each override with the value it replaced. So the MCP server resolves nothing, and a pin on such a language works like any other pin.
- Decided: an edit of the document, or an accepted proposal, becomes overrides (the difference from the base). When the base gets a new version, a language that follows the latest is built again; an override that no longer fits is skipped and listed as a review note.

**8. Better references**
- Accept STL, STEP and 3MF files with auto-rendered views.
- Extract and show colour palettes.
- Add board search to the app (`search_items` is MCP-only today).
- Done: model files (also OBJ) on the Board, a palette for each image and for the board, and a search field on the Board.
- Decided: a model becomes one image with four views (two three-quarter views, front and top) in neutral clay, and the file stays in the library. The render shows form and proportion; the analysis is told that it shows no colour or material.

## 3.4 Plumbing and workflow

**9. Importers**
- Browser extension and Are.na or Pinterest board import.
- Done: **Import board…** on the Board, and the extension in `extension/` (Chrome, Edge, Brave, Arc) with **Settings › Browser extension**.
- Decided: the extension talks to the app through a bridge on this Mac only (127.0.0.1) with a token, and the bridge is off until you turn it on. Are.na gives every image of a public channel. Pinterest has no open API, so the import reads the public feed of a board, which holds the newest 25 pins; the extension covers the rest.
- Not done: the extension is not in a browser store. Load it unpacked.

**11. Export formats**
- CSS variables, Figma tokens and a Grasshopper parameter JSON.
- Done: **Export…** in the Document tab and the MCP tool `export_tokens`.
- Decided: the Figma export uses the W3C design-tokens form, which the common token plugins read. A measure with a range gives a minimum and a maximum; a process-dependent measure takes its value from the hard rules.

**12. Cost and safety**
- Token cost estimate before each pass.
- "Re-analyze all" when the model changes.
- Library snapshot and export, as a safety net beside the warning about synced folders.
- Done: an estimate next to the queue and before **Analyze all again**, a notice when analyses come from another model, and **Snapshots** in Settings › Library location.
- Decided: the estimate uses the averages of earlier calls in this library, and built-in averages until there are three calls. It shows dollars only when you enter prices for the provider, because prices change and the app does not look them up.
- Decided: a model change never starts work by itself. The app shows which analyses another model wrote and what a new analysis would cost; you start it.
- Decided: a snapshot is one zip file with the database and every file. The app also keeps a copy of the database alone for each of the last seven days.

**13. Usage insight**
- Record which rules and families agents fetch.
- Show unused parts of a language and recurring amendment proposals.
- Done: **What agents use** in the Insight tab.
- Decided: the MCP server writes one small record for each call that reads or checks a language. The records stay in the library on this Mac. Use before this version is not counted.

## Open questions

- Should a vision-model check (render against the references) join the number check later? Still open.
- Is inheritance (7) worth the resolution complexity, or is copy-and-edit enough? Answered: it is built without resolution at read time; see item 7.
- Does the library schema need a migration for outputs, and how does that interact with the library move? Answered: new tables and columns are added when the app opens a library; an older app ignores them. The move copies the library as files, so it needs nothing more.
- Should the geometry check also read the sliced file (G-code or a sliced `.3mf`), to judge what the slicer really made of a thin wall?
- Should model references keep their colours when the file has them (3MF, OBJ with materials)?

## Not in Phase 3

- Cloud sync of the library.
- Multi-user editing.
