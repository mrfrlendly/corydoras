# Corydoras

Corydoras is a macOS app that turns mood boards into design languages for agents.

You collect reference images in a language. A vision model analyzes each image. A second pass writes a versioned language document (`language.md` + `tokens.json`). Agents in Fusion 360, Blender, Rhino, or a concept-image tool read the document through an MCP server.

## Install

Double-click **Install Corydoras.command** in Finder, or run:

```bash
bash scripts/setup-mac.sh
```

The script does these steps:

1. It checks Xcode Command Line Tools, Rust (1.85 or newer), and Node.js (20 or newer).
2. It installs the frontend packages.
3. It builds the MCP server and copies it to `~/.local/bin/corydoras-mcp`.
4. It builds `Corydoras.app` and copies it to `~/Applications`.
5. It opens the app.

To also add the MCP server to Claude Desktop, run `bash scripts/setup-mac.sh --register-claude`. The script makes a backup of `claude_desktop_config.json` first.

## First launch

1. Select the library folder (default `~/Corydoras`).
2. Open **Settings**. Under **Model providers**, open a provider, paste its API key, and click **Test**. The key is saved in a file that only your user account can read.
3. Choose the provider for **Images and sources** and the provider for the **Language document**.
4. Click **+** next to **Languages**, type a name, and press Return.
5. Drag images into the board, or paste a screenshot with ⌘V. A video (MP4, MOV, M4V) adds its key frames.

API keys are files in `~/Library/Application Support/Corydoras/keys/` (folder mode 0700, files mode 0600), so a rebuild never asks for your password. Keys that earlier versions saved in the Keychain move into these files the first time the app uses them; macOS can ask for Keychain access that one last time.

## How it works

| Step | What happens |
| --- | --- |
| Add an image | The app copies the file into the library, makes a thumbnail, and queues pass 1. |
| Pass 1 (item analysis) | The model gets the full image, one crop for each region mark, and your annotations. It returns the object family, features, measurements (each with "of", unit, basis and confidence), object colours, and presentation colours. It gives no manufacturing values. Thinking is off. |
| Add a source | A link or a document (tutorial, article, PDF). The app reads the text and a preview, and queues a source analysis. |
| Source analysis | The model reads the text (up to 60 000 characters) and returns principles, techniques, values with units, definitions, do and do-not lists, relevance and limits. Thinking is off. |
| Pass 2 (synthesis) | After no item changes for the debounce time (default 60 s), the model reads all item and source analyses and the previous version. It returns a structured spec (schema 2). At least one analyzed image is necessary. |
| Validation | The app checks the structure. Errors go back to the model. |
| Review | The app looks for numbers in text fields, unknown evidence IDs, observed values without evidence, and exceptions that point at no rule. The findings go back to the model one time. Findings that remain are saved with the version and shown at the top of the document. |
| Version | The app renders `language.md` from `tokens.json`, so the two cannot disagree. If the inputs did not change, the app saves no version. |
| Your edit | In the Document tab you edit `tokens.json`. The same validation applies, and the app renders a new `language.md`. |

## Videos

Drag a video onto the Board, or choose it with **Add images…**. The app reads it with AVFoundation (macOS) and adds up to 20 key frames as images. They are analyzed like any other image.

- A key frame is a distinct view, not a codec frame. The app scans the video with small samples (every 0.5 s, or at most 300 samples), starts a new segment at each cut or when a pan has changed the picture enough, and keeps the middle frame of each segment at up to 2048 px.
- Dark or flat frames (fades), one-sample transitions and repeats of an earlier view are left out. With more than 20 segments, the longest ones win.
- Each frame is named after the video and its position, for example `demo · 0:12.jpg`, and that is also its source.
- macOS reads MP4, MOV and M4V. Convert WebM, MKV or AVI first, for example with QuickTime Player › File › Export As.
- The app does not keep the video.

## Spec format (schema 2)

| Part | Content |
| --- | --- |
| `principles` | Rules for every family, with priority, basis (observed, designer, inferred, source), and evidence IDs |
| `definitions` | Terms that measures use, for example pitch = centre-to-centre distance |
| `families` | Object families (for example "Blade field") with `applies_to`, measures, and family rules |
| measures | Value or range, unit, `of` (exactly what is measured), basis (observed, preferred, process-dependent), confidence, evidence |
| `exceptions` | Which ID they override, scope, condition, replacement, and priority |
| `cmf.object` / `cmf.presentation` | Product colours and materials, separate from background, light, and shadow colours |
| `prompts` | Positive and negative fragments, plus fragments for each family |

A process-dependent measure has no value in the language. A hard rule with the same name supplies it.

## Model providers

Corydoras works with your own API key from any of these providers, or with a model on your Mac:

| Provider | Format | Note |
| --- | --- | --- |
| DeepSeek | OpenAI-compatible | Thinking mode for the language document |
| OpenAI | OpenAI-compatible | Thinking sets `reasoning_effort` |
| Anthropic (Claude) | Messages API | Adaptive thinking; budget thinking for models before 4.6 |
| Google Gemini | OpenAI-compatible endpoint | Thinking sets `reasoning_effort` |
| OpenRouter | OpenAI-compatible | One key for many providers |
| Mistral, xAI | OpenAI-compatible | |
| Ollama, LM Studio | OpenAI-compatible, on this Mac | No key. Set a context length of 32 k or more. |
| Other | OpenAI-compatible or Messages API | Any address |

- Each provider keeps its own key file. The provider settings are app settings, so they stay the same when you move or switch the library.
- **Images and sources** need a model that reads images. **Test** sends a small red image and records whether the model reads it.
- **Load models** asks the provider for its model list.
- **Output limit** caps the answer size. A model with a small limit needs a small value.
- When a server refuses a parameter (for example `temperature` or a thinking option), Corydoras removes or changes that parameter and sends the call again. It remembers the change for that model until the app quits.
- The 0.4 DeepSeek setting becomes the first provider, with the same key.

## Library location

**Settings › Library location** shows the folder and has these controls:

| Control | What it does |
| --- | --- |
| Show in Finder | Opens the folder |
| Move library… | Stops the queue, copies the library to the new folder, checks the copy, and then uses it. If you choose a folder with files in it, the library goes into a new "Corydoras" folder inside it. The old folder stays, or goes to the Trash if you select that option. A job that the move stopped runs again after the move. |
| Open another library… | Switches to a library folder, or starts a new library in an empty folder. The app lists the libraries that it opened before. |

Do not keep the library in iCloud Drive, Dropbox, Google Drive or OneDrive: a sync can damage the database while the app writes to it. The app shows a warning for these folders.

## Sources

The Sources tab of a language holds links and documents that teach technique: tutorials, articles, videos and specifications.

| Input | What the app reads |
| --- | --- |
| Web page | The main text, the title, the description, the site name and the preview image (`og:image`) |
| Video (YouTube) | Only the title, the channel and the description. The app cannot read the speech. |
| Link to a PDF or a text file | The file. The app keeps a copy. |
| PDF | The text. A PDF with little or no text (a scan, or a drawing sheet) goes to the model as page images, up to 8 pages. This needs a provider that reads images. |
| TXT, MD, CSV, HTML | The text |
| DOC, DOCX, RTF, ODT | The text, through the macOS `textutil` tool |

To add a source:

1. Open a language and click the Sources tab.
2. Paste a link in the field and push Return, or click Add documents….
3. Alternatively, drag documents or browser links onto the tab. On the Board, a dropped document or page link also goes to Sources.
4. Click a card to see the preview, the extracted text and the notes from the model.
5. Write a note to tell the model what to take from the source. Set the weight from 1 to 5.

Rules from a source have the basis *source* and cite the source ID. When a source and the images disagree about the look, the images win.

## Rule sets

Rule sets hold hard rules, for example 3D-printing limits. They are in the sidebar under **Rule sets**.

- A rule with a value is a limit (minimum wall 1.2 mm). A rule without a value is an instruction (orient load paths along the layers).
- Hard rules win over every language principle, measure, exception and prompt. Agents get them first.
- A project selects rule sets with checkboxes. A rule set marked **Apply to every project** also applies to agents that read a language without a project.
- When a language marks a measure process-dependent, the brief takes its value from the rule with the same name.

The app starts with "FDM · PETG · Bambu P1S" (12 limits and 5 instructions). Its values are common starting points: check them against your slicer profiles and test prints. A 0.2 process profile becomes a rule set when you open the library in 0.3.

## Activity and cancel

Click the status line at the bottom of the sidebar to open **Activity**:

- **Now** shows the running job as steps, the live state of the model call (in the provider's queue, thinking, writing), the output tokens so far, the time, and the time since the last data. **Cancel this job** stops the call at once.
- **Up next** lists the waiting jobs, with earlier errors. Each has a **Cancel** link.
- **Recent activity** explains each attempt, cut-off, repair, review and failure in plain words.
- **Pause queue** lets the running job finish and starts no new job. The setting stays after a restart.

Calls stream from the provider. A call with no data for 2 minutes stops, and the queue tries it again later. Only network, rate-limit and server errors go back into the queue; an error that would repeat stops the job and appears in the log.

## Library layout

```text
~/Corydoras/
  library.sqlite                index, annotations, analyses, jobs, projects
  languages/<id>/
    items/                      original images
    thumbs/                     thumbnails
    crops/                      region crops that pass 1 sent
    sources/                    documents, previews (.jpg) and extracted text (.txt)
    versions/v0001/
      language.md
      tokens.json
    _removed/                   files of removed items
```

## MCP server

The server is a separate binary that uses the stdio transport. It reads the library in query-only mode, so it works when the app is closed. Its one write is `propose_amendment`, which only adds a proposal for you to review.

| Tool | Returns |
| --- | --- |
| `list_languages` | ID, name, latest version, item count, and families of each language |
| `list_projects` | Projects and their pins |
| `get_project_brief` | Order of authority, the hard rules, resolved process-dependent values, and the pinned versions limited by scope |
| `list_rule_sets` | Hard rule sets, their rules, and whether they apply everywhere |
| `get_language` | The document and `tokens.json`; with `family`, only that family and the rules that apply to it |
| `get_sources` | Tutorials and documents of a language with the notes from each; with `ids`, only those sources |
| `get_reference_images` | Images with the highest weight, or exact items by `item_ids` (the 8-character evidence IDs) |
| `search_items` | Items whose tags, notes, region notes or analysis match the query words, best first, with the matching text and evidence IDs (no images) |
| `propose_amendment` | Stores a proposed change to the latest spec of a language for your review; the language does not change |

### Proposed amendments

An agent can call `propose_amendment` with a JSON Patch (`add`, `remove`, `replace`) on `tokens.json`. A path segment into an array can be an index, `-` (append), or the `id`, `name` or `term` of an element, for example `/families/blade-field/measures/blade pitch/value`.

- The server applies the patch to the latest version and runs the same validation as your own edit. An invalid patch, or a `base_version` that is not the latest version, is refused, and the agent gets the reason.
- The Document tab shows open proposals with each change (old value → new value), the agent's reason and the client name. The tab shows how many proposals are open.
- **Accept as new version** applies the patch to the latest version and saves it as an edited version. A proposal that no longer applies, for example after a new version, shows why and can only be rejected.
- A language holds at most 20 open proposals.

Without `--library`, the server reads the library that the app uses, also after a move. With `--library <path>`, it always reads that folder.

Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "corydoras": {
      "command": "/Users/<you>/.local/bin/corydoras-mcp",
      "args": []
    }
  }
}
```

Claude Code:

```bash
claude mcp add corydoras -- ~/.local/bin/corydoras-mcp
```

## Development

```bash
npm install
npm run tauri dev                        # app with hot reload
CORYDORAS_LIBRARY=/tmp/lib npm run tauri dev   # same, on another library (the saved setting stays)
cargo test -p corydoras-core             # unit tests
CORYDORAS_API_KEY=… cargo run -p corydoras-core --example e2e -- /tmp/lib https://api.deepseek.com
```

## Phase 2

- Menu bar icon for capture
- Canvas view
