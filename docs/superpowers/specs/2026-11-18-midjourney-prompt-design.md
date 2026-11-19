# Midjourney prompt in the language document

Status: approved and built on 2026-11-18, on branch `midjourney-prompt`. See "Changes from the review" at the end.

## Intent and the why

The designer makes concept art in Midjourney before modelling. Today the language document has an "Image-generation tokens" section: long comma-separated positive and negative lists. Midjourney's documentation (read 2026-11-18; the default model is V8.2) says the opposite works: short phrases that describe what you want, no long lists, and exclusions in `--no`. So the lists do not paste in well, and concept art drifts from the language.

The goal is **a prompt that pastes into Midjourney and gives concept art that matches the language**. The AI writes it in the same pass that writes the rest of the document.

## Goals

1. Each new language version holds a Midjourney prompt for the whole language and one for each family.
2. Each prompt starts with a slot, `[your object]`. The designer types the object; the language supplies the look.
3. The prompt is ready to paste: correct Midjourney syntax, parameters at the end.
4. It shows in the document, in the family brief, in the `prompts` pin scope, in the version comparison, and to agents through the MCP server.
5. The document view has a copy button on each prompt.

## Non-goals

- Sending prompts to Midjourney or making images in the app.
- Style references (`--sref`) from the reference images. Midjourney needs hosted image URLs.
- Aspect ratio, stylize, seed or version flags. The designer's Midjourney settings decide these.
- A prompt for each project or each output.
- Changing or removing the existing token lists. Other tools read them.
- Adding the prompt to versions that exist today. A language gets it with its next version.

## Anti-goals (failures even if the feature "works")

- The prompt says something the spec does not (a colour, a finish, a form).
- The prompt uses words only this spec defines (a family name, "G2 Organic"), which Midjourney cannot know.
- The model invents parameters, or puts exclusions in the sentence ("without chrome").
- A version or a hand edit that has no Midjourney prompt fails validation.

## Constraints (Midjourney documentation, read 2026-11-18)

- Short, simple prompts work best. Describe what you want to see. Do not write long lists or instructions.
- Useful parts: subject, medium, environment, lighting, colour (in words), mood, composition.
- `--no` takes a comma-separated list. Midjourney's moderation reads each word alone ("modern clothing" is read as "modern" and "clothing").
- Parameters go at the end, with no punctuation in them.
- `--raw` turns off Midjourney's own styling, so the image follows the prompt more closely. It works on V5.1 and later.

## Approaches considered

| | What it is | Trade-off |
|---|---|---|
| A | The app builds the prompt from the existing token lists | No model cost. But it only rearranges long lists, the AI does not write it, and the app cannot turn hex codes into colour words. |
| **B (chosen)** | The synthesis pass writes the prompt text as a new part of the spec; the app adds the slot and the parameters | One pass, so the prompt and the spec cannot disagree. The app guarantees the syntax. A language needs a new version to get it. |
| C | A separate model call after synthesis | Could be re-run alone, but it costs a second call each time and can drift from the spec. |

For what the prompt depicts, the designer chose the **object slot** on 2026-11-18 over ready-made example objects, which would pull every image toward one object. B is the builder's call; the designer handed over the how.

## The design

### Data (tokens.json, schema 2, no schema bump)

A new optional key under the existing `prompts`:

```json
"midjourney": {
  "language": "product concept render, one continuous matte white body, soft tangent surfaces, irregular rounded openings, soft studio light, neutral seamless background",
  "no": "chrome, visible frame, sharp edges",
  "families": { "blade-field": "A complete description for this family." }
}
```

- `language` describes the look that every family shares.
- Each `families` entry is complete on its own: it must work without `language`.
- `no` is one list for the whole language.
- The model writes no object name and no parameters.

### What the model is told (added to the synthesis instruction)

- Write a few short descriptive phrases in this order: medium, form and surfaces, material and finish, colour in words, light and background.
- At most 40 words for each text. No keyword lists, no hex codes, no dimensions, no terms that only this spec defines.
- Say what the image shows. Put what to leave out in `no`: at most 8 nouns for visible things. Each word of `no` is read alone, so use no word that is wrong on its own.
- Take the content from the spec: product colours from the CMF, light and background from the presentation colours.

### What the app does

- **Assembles the line:** `[your object], <text> --no <no> --raw`. An empty `no` leaves `--no` out.
- **Renders** a "Midjourney prompt" section after "Image-generation tokens": one line of instruction, then one fenced block for the whole language and one for each family. The family brief shows that family's prompt, or the language prompt when the family has none.
- **Repairs** model output: a list becomes one string; a slot, `/imagine` prefix or parameters that the model wrote are removed; a leading `--no` is removed.
- **Validates.** The key may be absent. If present, a wrong type is an error. Warnings: a family ID that does not exist, a family with no prompt, a hex code or `--` in the text, a text over 60 words. When the model gives no prompt, the document's "Check before use" note says so.
- **Compares versions:** a "Midjourney prompt" section in the changes list.
- **Inheritance:** a language that extends a base carries the base's prompt. An edit of tokens.json becomes an override, as for every other field.
- **Re-run:** the pipeline version goes from 3 to 4. Each language then writes a new version on its next update, also when its references did not change. Nothing runs automatically.

### Where the designer and agents see it

- **Document view:** every fenced block gets a Copy button.
- **MCP:** `get_language` and `get_project_brief` return the section in the document and the key in tokens.json. The tool descriptions and the server instructions say how to use it: replace `[your object]`, keep the rest.
- **README:** the tokens table and one short paragraph.

## Tests

- `spec.rs`: render (full document, family brief, `prompts` scope), repair, validate with and without the key.
- `diff.rs`: changes are listed. `inherit.rs`: an override of the prompt survives a rebuild.
- An existing fixture without the key stays unchanged and still passes: the backward-compatibility check.
- App: build and type-check, then a live check of the Copy button on a copy of the library.

## Left to the builder

The exact wording of the instruction and the messages, the look of the Copy button, and test names.

## Changes from the review (2026-11-18)

- **A manual version keeps the language due.** A hand edit, an accepted proposal or a restore used to record the current input fingerprint. After the pipeline went to 4, such a version made **Synthesize now** answer "no change", so the language got no prompt. A manual version now keeps the fingerprint of the version before it when that version is behind its inputs. Side effect: a hand edit made while a synthesis is already due no longer cancels that synthesis.
- **Hand edits get warnings, not repair.** Validation also warns when a text has a slot or a command, when a line break hides a parameter, and when `no` has a parameter. The line shows what was typed.
- **The prompt never blocks the document.** A text from the model that is missing or not text becomes empty with a warning. It is never a validation error, because an error costs a paid retry.

