# Corydoras

Corydoras is a macOS app that turns mood boards into design languages for agents.

You collect reference images in a language. A vision model analyzes each image. A second pass writes a versioned language document (`language.md` + `tokens.json`). Agents in Fusion 360, Blender, Rhino, or a concept-image tool read the document through an MCP server.

## Built with AI assistance

Corydoras is developed with a lot of help from AI coding assistants, mainly Anthropic's Claude (through Claude Code). Most of the code, tests and documentation were written by an AI under my direction: I decide what to build and how it should behave, review the changes, and test the app on my own Mac and library. I think it is fair to say this openly.

What this means for you:

- Treat it like any young open-source project: read the code before you depend on it, and keep backups of your library.
- The tests cover the core library, but not every path. If something breaks, please open an issue.
- Contributions are welcome, whether you write them by hand or with an assistant. Please test what you send.

This is separate from the app's own use of AI: Corydoras sends your reference images and sources to the model provider that you choose (or a local model) to analyze them. See [Model providers](#model-providers).

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
2. Open **Settings**. Under **Model providers**, open a provider, paste its API key, and click **Test**. The key is saved in a file that only your user account can read. With OpenRouter or a ChatGPT plan you can sign in instead of pasting a key: see [Account sign-in](#account-sign-in).
3. Choose the provider for **Images and sources** and the provider for the **Language document**.
4. Click **+** next to **Languages**, type a name, and press Return.
5. Drag images into the board, or paste a screenshot with ⌘V. A video (MP4, MOV, M4V) adds its key frames. A 3D model (STL, 3MF, OBJ, STEP) adds a render of four views.

API keys are files in `~/Library/Application Support/Corydoras/keys/` (folder mode 0700, files mode 0600), so a rebuild never asks for your password. Keys that earlier versions saved in the Keychain move into these files the first time the app uses them; macOS can ask for Keychain access that one last time. The sign-in of a ChatGPT plan is a file in the same folder.

## How it works

| Step | What happens |
| --- | --- |
| Add an image | The app copies the file into the library, makes a thumbnail, works out the main colours, and queues pass 1. |
| Pass 1 (item analysis) | The model gets the full image, one crop for each region mark, and your annotations. It returns the object family, the view (three-quarter, front, side, top, back or detail), features, measurements (each with "of", unit, basis and confidence), object colours, and presentation colours. It gives no manufacturing values. Thinking is off. |
| Add a source | A link or a document (tutorial, article, PDF). The app reads the text and a preview, and queues a source analysis. |
| Source analysis | The model reads the text (up to 60 000 characters) and returns principles, techniques, values with units, definitions, do and do-not lists, relevance and limits. Thinking is off. |
| Pass 2 (synthesis) | After no item changes for the debounce time (default 60 s), the model reads all item and source analyses and the previous version. It returns a structured spec (schema 2). At least one analyzed image is necessary. |
| Validation | The app checks the structure. Errors go back to the model. |
| Review | The app looks for numbers in text fields, unknown evidence IDs, observed values without evidence, and exceptions that point at no rule. The findings go back to the model one time. Findings that remain are saved with the version and shown at the top of the document. |
| Version | The app renders `language.md` from `tokens.json`, so the two cannot disagree. If the inputs did not change, the app saves no version. |
| Your edit | **Edit document** in the Document tab edits `tokens.json`. The same validation applies, and the app renders a new `language.md`. |
| The loop | Agents send renders of what they made (**Outputs**), check their numbers and model files against the rules, and you log how prints came out. Your verdicts and the print log go into the next version and the next brief. |

The estimate in the header of a language (for example **≈ 48 k tokens**) says what the waiting passes will use; see [Cost estimates](#cost-estimates).

## Canvas

The Board has two views: **Grid** (the masonry of cards) and **Canvas**, a free board where you place references yourself.

| Action | How |
| --- | --- |
| Pan | Drag the background, or scroll with two fingers |
| Zoom | Pinch, or ⌘-scroll; the zoom follows the pointer. The toolbar has − / + and a click on the percentage goes to 100 % |
| Move a card | Drag it |
| Resize a card | Drag its bottom-right corner |
| Open a card | Click it (the inspector opens as in the grid) |
| Fit all | The fit button in the toolbar |
| Arrange by weight | The grid button in the toolbar lays all cards out by weight; **Undo** in the message puts them back |

- The app saves each card's place in the library, so the arrangement stays after a restart and a library move. The view (pan and zoom) and the chosen mode are remembered per language on this Mac.
- New items, for example a drop, a capture or video frames, appear below the arrangement.
- The canvas is layout only: moving cards does not start an analysis or a new version.

## Screen capture

The fish in the menu bar captures part of the screen into a language. The keyboard shortcut does the same, from any app; the default is **⌃⌥⌘C** (Control-Option-Command-C).

1. Press the shortcut, or click the fish and choose **Capture to …**.
2. Drag over a region, press Space to pick a window, or press Escape to cancel.
3. The capture becomes an item of the capture language, named like `Screen capture 2026-11-08 at 09.15.03.png`, and the analysis starts.

- **Capture Into** in the menu, or **Settings › Screen capture**, sets the language. Without a choice, captures go to the language that changed last.
- In **Settings › Screen capture**, click the shortcut and press new keys (at least one of ⌃ ⌥ ⌘, or a function key). **Clear** removes it. A shortcut that another app holds is refused with a message.
- Closing the window keeps Corydoras running in the menu bar. **Quit Corydoras** in the menu (or ⌘Q) quits; a click on the Dock icon shows the window again.
- The first capture asks for Screen Recording access in System Settings › Privacy & Security. The app is ad-hoc signed, so macOS can ask again after an update.

## Videos

Drag a video onto the Board, or choose it with **Add images…**. The app reads it with AVFoundation (macOS) and adds up to 20 key frames as images. They are analyzed like any other image.

- A key frame is a distinct view, not a codec frame. The app scans the video with small samples (every 0.5 s, or at most 300 samples), starts a new segment at each cut or when a pan has changed the picture enough, and keeps the middle frame of each segment at up to 2048 px.
- Dark or flat frames (fades), one-sample transitions and repeats of an earlier view are left out. With more than 20 segments, the longest ones win.
- Each frame is named after the video and its position, for example `demo · 0:12.jpg`, and that is also its source.
- macOS reads MP4, MOV and M4V. Convert WebM, MKV or AVI first, for example with QuickTime Player › File › Export As.
- The app does not keep the video.

## 3D models

Drag an STL, 3MF, OBJ or STEP file onto the Board, or choose it with **Add images…**. The model becomes one reference image with four views: two three-quarter views, the front and the top. The card has a **3D** badge.

- The render is neutral clay without perspective. It shows form, proportion and edges, and the analysis is told that it shows no colour or material.
- The model file stays in the library next to the render. The inspector shows it under **3D model**.
- STL, 3MF and OBJ are read directly. A 3MF project that Bambu Studio saved works too.
- STEP (`.step`, `.stp`) needs [FreeCAD](https://www.freecad.org) on this Mac, in Applications or as `freecadcmd` on the PATH. Corydoras runs FreeCAD's console program with empty settings of its own, so a FreeCAD window that you have open is not touched. Without FreeCAD, a STEP file gets a message that says so.
- A file can be up to 400 MB and 6 million triangles.

## Search and colours

- **Search this board** finds references by their tags, notes, region marks and what the analysis saw. The best match comes first; hold the pointer over a card to see the text that matched. It is the same search that agents use (`search_items`).
- The strip above the cards shows the main colours of the board, counted by the weight of each reference. Click a colour to copy its hex value.
- The inspector shows the five main colours of one image under **Colours**. These come from the pixels, not from the model, so they are there before the analysis runs.

## Keyboard

| Keys | Where | What they do |
| --- | --- | --- |
| Return or Space | A card on the Board or in Sources | Opens or closes the card |
| ← → | Board, Sources | The previous or next card |
| ↑ ↓ | Board grid | The card above or below |
| 1–5 | Board, with a card open | Sets the weight |
| Escape | Inspector, confirmation | Leaves a field, then closes the inspector; cancels a confirmation |
| ⌘F | Board | Goes to **Search this board** |
| ← → Home End | The tabs of a language | Goes to the next tab |
| ⌘, | Anywhere | Opens Settings |

The tags and the note of an image save when you leave the field, or when you open another card.

## Import a board

**Import board…** on the Board takes the link of an Are.na channel or a Pinterest board.

| Service | What you get |
| --- | --- |
| Are.na | Every image of a public channel, up to 300. Blocks without an image (text, channels) are left out. |
| Pinterest | The newest 25 pins of a public board. Pinterest has no open interface for more; use the browser extension for the rest. |

- An image that the language has already is not added again, so you can import the same board later to get what is new.
- Each image keeps its page as the source.
- The import makes no account and needs no key. A private board cannot be read.

## Browser extension

The folder `extension/` holds an extension for Chrome, Edge, Brave and Arc. It adds two commands to the right-click menu: **Add image to Corydoras** and **Add page to Corydoras sources**, each with a list of your languages.

1. In **Settings › Browser extension**, select **Let the browser extension add references**. The app shows a port and a token.
2. In the browser, open `chrome://extensions`, turn on **Developer mode**, click **Load unpacked**, and choose the `extension` folder.
3. Open the options of the extension and paste the port and the token.
4. Right-click an image or a page. Without a choice of language, it goes to the language that screen captures go to.

The extension talks to a small bridge inside the app. The bridge listens on this Mac only (127.0.0.1), answers only requests that carry the token, and is off until you turn it on. Corydoras must be running; a closed window is enough. **New token…** asks first, then makes the old token useless: the extension stops working until you paste the new token.

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
| `extends` | Only in a language with a base: the base, its version, and each change with the value that it replaced (see [Inheritance](#inheritance)) |

A process-dependent measure has no value in the language. A hard rule with the same name supplies it.

## Versions

The **Versions** tab lists every version of a language with its date, model and item count, and the projects that pin it.

| Control | What it does |
| --- | --- |
| A / B | Chooses the two versions to compare |
| **Changes** | Says in words what changed from A to B, by section: measures (old value → new value), rules, principles, families, colours, prompts, do and do not, and references |
| **language.md**, **tokens.json** | The same comparison as a line-by-line text difference |
| **Open** | Shows that version in the Document tab |
| **Pin in…** | Makes a project use that version at once; the message has **Undo**. A project keeps its pin when the language gets newer versions. |
| **Restore** | Rolls the language back: saves that version again as the newest one. The versions in between stay in the list, so you can go forward again. |

Agents get the same list of changes with `compare_versions`, for example to see what to redo after you updated a language.

## Inheritance

A language can extend another language and change a few rules, for example "2000s Industrial PETG" on top of "2000s Industrial".

1. Make a language, click **Edit details**, and choose the base under **Extends**. Choose **Follow the latest version**, or one version of the base.
2. Open the **Document** tab. The document is the base document. Edit it with **Edit document**; each difference from the base becomes a change of this language.
3. The panel above the document lists the changes (old value → new value). The **×** on a change removes it, so the value of the base applies again.

- A language with a base has no synthesis of its own, and its Board is optional. Evidence IDs point at the references of the base.
- Each version is a complete document. It names the base, the base version and each change in `extends` and in an **Inheritance** section, so an agent or a project pin reads it like any other language and resolves nothing.
- When the base gets a new version and the language follows the latest, the app builds a new version of the language. A change that no longer fits (its place in the document is gone) is skipped and listed as a review note. With a fixed base version, the panel offers **Update to vN**.
- An amendment that you accept on such a language becomes a change in the same way.
- A base can have a base. A language cannot extend itself or one of its own descendants.

## Insight

The **Insight** tab has two reports. Neither needs a model call.

**Coverage** lists what the language cannot answer yet, with a suggestion for each gap:

- a family that rests on one reference, has no "not this" reference, or has no measures or rules;
- references that are not analyzed yet;
- views that no reference of a family shows (front or side, detail);
- a measure that one reference with low confidence backs, or references that disagree on a measure or a finish;
- a process-dependent measure that no hard rule fills;
- references that no rule cites, evidence that points at removed items, and review notes of the last synthesis.

Agents get the report with `get_coverage`, and are told not to invent a value where the language has a gap.

Each gap has a link to the place where you fix it: **Show on the Board** selects the reference that the gap names, and the reference IDs in a gap open their card too. The gaps that are only worth a look start folded.

**What agents use** shows the calls of the MCP server for this language in the last 30, 90 or 365 days: the calls per tool and client, which families agents asked for by name and checked, which measures a check used, which references agents asked to see, and the places in the document that got more than one amendment proposal. A family that no call names is marked **not used**. The server notes one small record for each call in the library on this Mac; use before this version is not counted.

## Export

**Export…** in the Document tab saves the measures and colours of the version that you see:

| Format | File | Content |
| --- | --- | --- |
| CSS variables | `<language>-v3.css` | `--…` custom properties under `:root` |
| Figma tokens | `<language>-v3.tokens.json` | Design tokens in the W3C form, which the common token plugins for Figma read |
| Grasshopper | `<language>-v3.grasshopper.json` | A parameter list with value, minimum and maximum for sliders, the colours as hex and RGB, and the hard limits |

A measure with a range gives a minimum and a maximum. A process-dependent measure takes its value from the hard rules that apply everywhere. Agents get the same files with `export_tokens`; with a project, the rule sets of that project fill those values.

## Model providers

Corydoras works with your own API key from any of these providers, with an account sign-in for two of them, or with a model on your Mac:

| Provider | Format | Note |
| --- | --- | --- |
| DeepSeek | OpenAI-compatible | Thinking mode for the language document |
| OpenAI | OpenAI-compatible | Thinking sets `reasoning_effort` |
| ChatGPT plan (OpenAI) | Responses API | No key: sign in with ChatGPT. The calls use the usage of your plan |
| Anthropic (Claude) | Messages API | Adaptive thinking; budget thinking for models before 4.6 |
| Google Gemini | OpenAI-compatible endpoint | Thinking sets `reasoning_effort` |
| OpenRouter | OpenAI-compatible | One key for many providers. Sign in, or paste a key |
| Mistral, xAI | OpenAI-compatible | |
| Ollama, LM Studio | OpenAI-compatible, on this Mac | No key. Set a context length of 32 k or more. |
| Other | OpenAI-compatible or Messages API | Any address |

- Each provider keeps its own key file. The provider settings are app settings, so they stay the same when you move or switch the library.
- **Images and sources** need a model that reads images. **Test** sends a small red image and records whether the model reads it.
- **Load models** asks the provider for its model list.
- **Output limit** caps the answer size. A model with a small limit needs a small value.
- When a server refuses a parameter (for example `temperature` or a thinking option), Corydoras removes or changes that parameter and sends the call again. It remembers the change for that model until the app quits.
- The 0.4 DeepSeek setting becomes the first provider, with the same key.

### Account sign-in

Two providers let you sign in with your account instead of pasting an API key. The sign-in opens in your browser, so Corydoras never sees your password. The app waits five minutes for the browser; **Cancel** stops the wait.

**OpenRouter.** Open the OpenRouter provider and click **Sign in with OpenRouter**. OpenRouter asks you to allow the app (its page names the app by its address on your Mac, `localhost`), makes an API key in your account and gives it to Corydoras. From then on the provider works as with a pasted key, and the calls use your OpenRouter credits. To take the access away, delete the key at openrouter.ai/settings/keys. **Remove key…** deletes only the copy on your Mac.

**ChatGPT plan.** Add the provider **ChatGPT plan (OpenAI)** and click **Continue with ChatGPT**. This is OpenAI's "Sign in with ChatGPT" for open-source apps, which OpenAI calls a preview. The calls use the usage that is included in your ChatGPT plan or credits balance, not an API account.

- The account needs a Go, Plus, Pro or Business plan. A Free account passes the browser part, and then the app says that ChatGPT did not accept the sign-in code: OpenAI refuses the account at that step and gives no reason.
- After the sign-in, the app loads the models of your plan and takes the first one. **Load models** lists them again; choose another and save the provider.
- One ChatGPT account serves every ChatGPT plan provider in the app. **Use another account** registers Corydoras for another account.
- Corydoras gets no access to your ChatGPT conversations. It keeps the tokens in `~/Library/Application Support/Corydoras/keys/chatgpt-plan-account` (mode 0600) and the registration of the app for your account in `chatgpt-sign-in` beside it. It renews the access token when it is old (it lives one hour) and sends it to `api.openai.com` only.
- **Manage usage** opens chatgpt.com/settings/usage, where you see and limit the usage. On a Plus plan, all apps that use your plan share the five-hour limit, so a large analysis pass takes from the same allowance as ChatGPT itself.
- When the limit is reached, the queue pauses itself and the jobs stay in it. **Activity** says "Usage limit reached". Resume the queue when your plan has usage again. The same happens when the sign-in ends, for example after you disconnect Corydoras in ChatGPT: sign in again, then resume the queue.
- The plan route takes no output limit and no temperature, and tokens have no price there. So the provider has no such fields, and its cost estimates show tokens only.
- **Sign out…** ends the sign-in at OpenAI and deletes the tokens on your Mac. When you remove the last ChatGPT plan provider, the app signs out too.

Anthropic and Google do not allow other apps to use the sign-in of a Claude or Gemini subscription, so these providers work with an API key only.

### Cost estimates

The app says what a pass will use before it runs:

- The chip in the header of a language (for example **≈ 48 k tokens**) is the sum of the passes that wait for this language. Hold the pointer over it for the count of image and source analyses.
- **Analyze all again** opens a bar under the header with the estimate. Nothing starts until you click **Analyze N again** there; Escape or **Cancel** closes the bar.
- **Synthesize now** makes no model call when nothing changed since the last version.
- The numbers are averages of earlier calls in this library. Until a pass has three calls, the app uses typical values.
- To see money next to the tokens, open a provider and enter its prices in US dollars per million input and output tokens. The app does not look prices up.

When you change the model for images and sources, the analyses that another model wrote stay. The language shows a notice with their count and the names of the models. **Analyze the N again…** opens the same bar with the estimate. The **×** hides the notice until the count or the model changes.

## Library location

**Settings › Library location** shows the folder and has these controls:

| Control | What it does |
| --- | --- |
| Show in Finder | Opens the folder |
| Move library… | Stops the queue, copies the library to the new folder, checks the copy, and then uses it. If you choose a folder with files in it, the library goes into a new "Corydoras" folder inside it. The old folder stays, or goes to the Trash if you select that option. A job that the move stopped runs again after the move. |
| Open another library… | Switches to a library folder, or starts a new library in an empty folder. The app lists the libraries that it opened before. |

Do not keep the library in iCloud Drive, Dropbox, Google Drive or OneDrive: a sync can damage the database while the app writes to it. The app shows a warning for these folders.

### Snapshots

| Control | What it does |
| --- | --- |
| Save a snapshot… | Writes one zip file, `Corydoras snapshot <date> <time>.zip`, with the database and every file of the library, into a folder outside the library. Keep it on another disk. |
| Open a snapshot… | Unpacks a snapshot into an empty folder and switches to it as the library. Your other library stays where it is. |

The app also keeps a copy of the database alone for each of the last seven days, in `.snapshots/` inside the library. These copies protect what the database holds (the index, annotations, analyses, projects and rule sets) against damage; they do not hold the images or the version files. API keys are never part of a snapshot or a copy.

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
5. Write a note to tell the model what to take from the source, and click **Save note** (the model reads the source again). The title saves when you leave the field. Set the weight from 1 to 5.

Rules from a source have the basis *source* and cite the source ID. When a source and the images disagree about the look, the images win.

## Rule sets

Rule sets hold hard rules, for example 3D-printing limits. They are in the sidebar under **Rule sets**.

- A rule with a value is a limit (minimum wall 1.2 mm). A rule without a value is an instruction (orient load paths along the layers).
- Hard rules win over every language principle, measure, exception and prompt. Agents get them first.
- A project selects rule sets with checkboxes. Changes on a project page apply at once; the message has **Undo**. A rule set marked **Apply to every project** also applies to agents that read a language without a project.
- When a language marks a measure process-dependent, the brief takes its value from the rule with the same name.

The app starts with "FDM · PETG · Bambu P1S" (12 limits and 5 instructions). Its values are common starting points: check them against your slicer profiles and test prints. A 0.2 process profile becomes a rule set when you open the library in 0.3.

### Import a slicer profile

The button with the file icon next to **Rule sets** makes a rule set from a Bambu Studio or OrcaSlicer profile.

1. Choose a process profile. The printer profile and the filament profile are optional. The lists hold the profiles that the slicers installed and your own profiles; the filter narrows a list, and **File…** takes a profile from another folder.
2. Alternatively, click **From a project file (.3mf)…**. A project that you saved in the slicer holds all three profiles with every value.
3. Read the rules and the comparison with the built-in set, change the name if you want, and click **Make rule set**.

A profile says how the printer lays lines down. The limits for a design follow from that, and each rule says how it was worked out:

| Rule | From the profile |
| --- | --- |
| Minimum wall | Two outer wall lines |
| Load-bearing wall | All wall loops of both sides |
| Minimum standing feature | One line with the Arachne wall generator, two lines with the classic one |
| Embossed or engraved detail | Three layers |
| Layer height | The layer height |
| Maximum unsupported overhang | 90° minus the support threshold angle |
| Maximum bridge | The bridge length without support, when the profile sets one |
| Elephant foot compensation | The value of the profile |
| Maximum part size, Nozzle diameter | The printer profile |
| Material, Hole compensation | Instructions, from the filament profile and the hole compensation |

- **Against the built-in set** lists each value next to the value of "FDM · PETG · Bambu P1S", with the difference.
- A profile holds only the values that differ from its parent, so the app follows the chain of parents. When a parent is missing, the screen says which values can be missing.
- The option **Also copy the built-in rules that a slicer profile cannot give** adds clearances, hole oversize and the instructions, each marked as copied.

### Print feedback

Each rule set has a **Print feedback** panel: what happened when a design was printed. An entry has a result (failed, warped, frosted, fine or other), a material, optionally the rule that it is about with the value that was printed, and a note.

- Agents get the entries with the hard rules, in a table after the rules.
- The check (`check_against_language` and `check_geometry`) gives WARN instead of PASS when a number is inside a limit but at or past a value that failed in a real print.
- The panel shows a hint when the prints disagree with a limit, for example "A print failed at 1.3 mm, which the limit of 1.2 mm allows. Set the limit above 1.3 mm." The log never changes a rule; you do.
- **Log a print…** on an output in the Outputs tab adds an entry for that design. An agent can add one with `log_print_result` when you tell it how a print came out; its name shows on the entry.
- Each library gets one entry to start with: clear and translucent PETG prints frosted and almost opaque. Remove it if it is not true for your printer.

## Geometry check

Corydoras can measure a model file and judge it against the hard rules.

- In the app: in the **Outputs** tab, set the **Build direction** of an output and click **Check a model…**. The report stays with the output.
- For an agent: the MCP tool `check_geometry` with the path of the file. With `output`, the report is saved with that output.

| Measured | How | Rule that judges it |
| --- | --- | --- |
| Thinnest wall | The material between two faces that are near-parallel. A chamfer or a taper is reported separately, not as a thin wall. | minimum wall |
| Steepest overhang | The angle from the vertical, against the build direction. Faces in the first two layers do not count. | maximum unsupported overhang |
| Longest bridge | A flat ceiling with material at both ends. A flat ceiling that is held at one end only is an overhang of 90°. | maximum bridge |
| Size | The largest side of the bounding box | maximum part size |
| Clearance | The smallest open gap between two bodies. Faces that touch count as contact, and bodies that overlap are reported. | sliding clearance, or the rule that you name (for example press-fit clearance) |

- The report also gives the triangle count, the bodies and cavities, open edges, the volume and the surface area, and where on the model each extreme is.
- The default build direction is +z. A file with more than one body is an assembly, so it gets overhang verdicts only when you give the build direction.
- The overhang limit and the layer height come from the rule sets of the project (or the rule sets that apply everywhere); without them the check uses 45° and 0.2 mm.
- An agent can send numbers from another tool in `measurements`, for example a wall thickness from ScanRuler. A sent number replaces the measured one, and the report says so.
- The file formats and the FreeCAD note for STEP are the same as for [3D models](#3d-models).
- The check measures the model, not the sliced file: it does not know what your slicer does with a wall that is thinner than two lines.

## Activity and cancel

Click the status line at the bottom of the sidebar to open **Activity**:

- **Now** shows the running job as steps, the live state of the model call (in the provider's queue, thinking, writing), the output tokens so far, the time, and the time since the last data. **Cancel this job** stops the call at once.
- **Up next** lists the waiting jobs, with earlier errors. Each has a **Cancel** link.
- **Recent activity** explains each attempt, cut-off, repair, review and failure in plain words.
- **Pause queue** lets the running job finish and starts no new job. The setting stays after a restart.
- **Queue: Auto / Manual** at the bottom of the sidebar. In Manual mode, jobs wait until you click **Run queue** there.

Calls stream from the provider. A call with no data for 2 minutes stops, and the queue tries it again later. Only network, rate-limit and server errors go back into the queue; an error that would repeat stops the job and appears in the log. A ChatGPT plan that has no usage left pauses the queue and keeps the jobs (see [Account sign-in](#account-sign-in)).

## Library layout

```text
~/Corydoras/
  library.sqlite                index, annotations, analyses, jobs, projects, rule sets,
                                print log, usage records
  .snapshots/                   a copy of the database for each of the last seven days
  languages/<id>/
    items/                      original images; for a 3D model, the render (<id>.png)
                                and the model file (<id>.model.stl, .3mf, .obj, .step)
    thumbs/                     thumbnails
    crops/                      region crops that pass 1 sent
    sources/                    documents, previews (.jpg) and extracted text (.txt)
    outputs/                    renders that agents sent
    versions/v0001/
      language.md
      tokens.json
    _removed/                   files of removed items
```

## MCP server

The server is a separate binary that uses the stdio transport. It reads the library in query-only mode, so it works when the app is closed. It never changes a language or a rule. It writes only what an agent hands in for you: a proposed amendment, an output, a saved check report, a print result, and one small usage record for each call (see [Insight](#insight)).

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
| `check_against_language` | Compares numbers that an agent measured on a design with the hard rules (first, and they win) and the measures of a family. Each number is PASS, FAIL, WARN or UNKNOWN; the check uses numbers only |
| `check_geometry` | Measures a model file (STL, 3MF, OBJ, STEP) and judges the thinnest wall, the steepest overhang, the longest bridge, the size and the clearance between bodies against the hard rules; see [Geometry check](#geometry-check) |
| `submit_output` | Stores a render of a design that the agent made, for your verdict in the **Outputs** tab (see below) |
| `log_print_result` | Adds an entry to the print log when you tell the agent how a print came out; see [Print feedback](#print-feedback) |
| `get_print_log` | The print log; with `project`, the entries that count for the rule sets of that project |
| `compare_versions` | What changed between two versions of a language, in words |
| `get_coverage` | What a language cannot answer yet, so that the agent does not invent a value there |
| `export_tokens` | The measures and colours of a version as CSS variables, Figma tokens or a Grasshopper parameter list |
| `propose_amendment` | Stores a proposed change to the latest spec of a language for your review; the language does not change |

### Outputs

An agent that made a design from a language sends a render with `submit_output` (an image file by path, or base64; PNG, JPEG or WebP up to 20 MB), optionally with the numbers it measured. The **Outputs** tab of the language lists them.

- **Accept** turns the output into a reference of weight 2, tagged `agent-output`. It supports the language but weighs less than your own references.
- **Reject** turns it into an anti-reference (weight 3) with your note, so the next version avoids it.
- After a verdict the card shows your note and **Undo verdict**, which removes the verdict and the reference it made. You can change your mind at any time; the next synthesis follows.
- Outputs without a verdict come first. A click on the render opens the full image. The trash button asks in the card before it deletes the output and the reference it made.
- Without a verdict, an output changes nothing. A language holds at most 50 outputs without a verdict.
- `check_against_language` can save its report with an output (`output` argument); the tab shows the result per measure.
- **Check a model…** measures a model file of the design and keeps the geometry report with the output. **Log a print…** records how the print of the design came out.

### Proposed amendments

An agent can call `propose_amendment` with a JSON Patch (`add`, `remove`, `replace`) on `tokens.json`. A path segment into an array can be an index, `-` (append), or the `id`, `name` or `term` of an element, for example `/families/blade-field/measures/blade pitch/value`.

- The server applies the patch to the latest version and runs the same validation as your own edit. An invalid patch, or a `base_version` that is not the latest version, is refused, and the agent gets the reason.
- The Document tab shows open proposals with each change (old value → new value), the agent's reason and the client name. The tab shows how many proposals are open.
- **Accept as new version** applies the patch to the latest version and saves it as an edited version. A proposal that no longer applies, for example after a new version, shows why and can only be rejected.
- **Reject…** asks first; Escape keeps the proposal.
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
cargo test -p corydoras-core             # unit tests of the library
cargo test -p corydoras                  # the app commands, called with the arguments that src/api.ts sends
CORYDORAS_API_KEY=… cargo run -p corydoras-core --example e2e -- /tmp/lib https://api.deepseek.com
cargo run -p corydoras-core --example geomcheck -- /tmp/lib part.step +z    # geometry report of a model file
cargo run -p corydoras-core --example boardcheck -- <link of a board>       # what a board import would read
```

| Variable | Effect |
| --- | --- |
| `CORYDORAS_LIBRARY` | The app and the MCP server use this library folder; the saved setting stays |
| `CORYDORAS_CONFIG_DIR` | The app keeps its settings and key files in this folder instead of `~/Library/Application Support/Corydoras`, for a test that must not touch your settings |
| `CORYDORAS_FREECADCMD` | The path of FreeCAD's console program, when it is not in Applications or on the PATH |

To run a test build next to the installed app, give it its own identifier, so that macOS treats it as another app:

```bash
npm run tauri build -- --debug --bundles app --config '{"identifier":"com.example.corydoras.dev","productName":"Corydoras Dev"}'
CORYDORAS_LIBRARY=/tmp/lib CORYDORAS_CONFIG_DIR=/tmp/config "target/debug/bundle/macos/Corydoras Dev.app/Contents/MacOS/corydoras"
```

## License

[MIT](LICENSE) © 2026 Travis Forsyth
