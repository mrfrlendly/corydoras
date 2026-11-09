# Corydoras roadmap: Phase 3

Starting point: version 0.6.0. Corydoras turns mood boards into versioned design languages, and agents read them through the MCP server together with hard rule sets.

**Phase 3 theme: close the loop.** Today the app feeds agents but never learns what they make. Phase 3 links agent output back to the language, checks it against the rules, and uses the result to improve the language.

## Order of work

| Milestone | Theme | Items | Depends on |
| --- | --- | --- | --- |
| 3.1 | Outputs and conformance | 1, 2 | none |
| 3.2 | Geometry and print feedback | 3, 4, 10 | 3.1 |
| 3.3 | Better languages | 5, 6, 7, 8 | 5 is independent; 6 and 7 benefit from 3.1 |
| 3.4 | Plumbing and workflow | 9, 11, 12, 13 | 13 needs 3.1 |

Items 5, 8, 9, 11 and 12 do not depend on the loop and can be picked up at any time to fill gaps.

## 3.1 Outputs and conformance (start here)

**1. Generated outputs as first-class items**
- Attach a render or model to a language version. Store it in the library next to `items/`.
- Mark each output accepted or rejected, with an optional note.
- A rejected output becomes an anti-reference. Pass 2 reads accepted outputs as supporting evidence and rejected ones as "do not" evidence.
- Done when: an output can be added from the app and from an MCP tool, and a rejection changes the next synthesis.

**2. Conformance-check MCP tool (`check_against_language`)**
- An agent submits measurements and/or a screenshot for a family.
- The tool returns each applicable rule as pass, fail or unknown, with the evidence IDs behind it.
- Hard rules are checked first and win over language rules.
- Read-only, like the other MCP tools, except that results can be saved as an output record.
- Done when: an agent can check a finished model and get a per-rule verdict.

## 3.2 Geometry and print feedback

**3. Geometry validation against hard rules**
- Check an STL or STEP against the active rule set: minimum wall, overhang, clearance.
- Reuse ScanRuler's thickness and clearance measurements where possible.
- Exposed as an MCP tool and as a button on an output.

**4. Print-feedback log**
- Record a print result (failed, frosted, warped, fine) against a rule or material.
- Feed the log into rule sets and into the conformance check.
- Seed it with the known finding that translucent PETG prints frosted.

**10. Slicer profile import**
- Create a rule set from a Bambu Studio or Orca profile.
- Show where the imported values differ from the built-in "FDM · PETG · Bambu P1S" set.

## 3.3 Better languages

**5. Version diff, rollback and pinning**
- Show a semantic diff between versions (for example "blade pitch 4.0 → 3.5, evidence added").
- Let a project roll back to or pin an earlier version.

**6. Coverage report**
- List what a language cannot answer: families with one reference, missing views, contradictions between items.
- Suggest what to add.

**7. Language inheritance and merge**
- A language can extend a base and override a few rules.
- Define how the MCP server resolves an inherited rule.

**8. Better references**
- Accept STL, STEP and 3MF files with auto-rendered views.
- Extract and show colour palettes.
- Add board search to the app (`search_items` is MCP-only today).

## 3.4 Plumbing and workflow

**9. Importers**
- Browser extension and Are.na or Pinterest board import.

**11. Export formats**
- CSS variables, Figma tokens and a Grasshopper parameter JSON.

**12. Cost and safety**
- Token cost estimate before each pass.
- "Re-analyze all" when the model changes.
- Library snapshot and export, as a safety net beside the warning about synced folders.

**13. Usage insight**
- Record which rules and families agents fetch.
- Show unused parts of a language and recurring amendment proposals.

## Open questions

- Which output formats count in 3.1: images only, or also STL and STEP from the start?
- Should the conformance check use the vision model, or only submitted numbers?
- Is inheritance (7) worth the resolution complexity, or is copy-and-edit enough?
- Does the library schema need a migration for outputs, and how does that interact with the library move?

## Not in Phase 3

- Cloud sync of the library.
- Multi-user editing.
