---
name: tpdf-reading
description: See what Manu is reading in tpdf, his terminal PDF viewer. Use when he says "this page", "this theorem", "this proof", "this question", "what I'm reading", or asks about a PDF he has open. For his handwriting, tablet, or "my work", use remarkable-view-screen instead.
---

# See what Manu is reading

1. Run this from the project root, with a kebab-case folder for the task:

   ```bash
   tpdf context --png ./.tmp/tpdf-reading/view.png </dev/null
   ```

   It prints JSON and writes a picture of exactly what's on his screen.

2. Read the PNG for anything with math. It's the source of truth, because
   equations extract badly as text. If he's zoomed in, it shows only that part.
3. Use `visible[].text` to quote or search. Use `page_text` for what surrounds
   the visible part of `page`.
4. Use `path` to find related files, such as the matching hints or solutions.

## Output

| Field | Meaning |
| --- | --- |
| `path`, `page`, `pages` | the file, the page at the center of the screen, the page count |
| `visible` | each visible page with its region in PDF points, top-left origin, and its text |
| `page_text` | all the text on `page` |
| `png` | absolute path of the picture, if `--png` was passed |
| `zoom`, `search` | `fit`, `width`, or a percentage, and his active search |
| `relation` | where the reader is relative to you, such as `same tmux session` |
| `seconds_since_change` | how long he has stayed on this view |
| `other_readers` | other open readers, as `pid`, `path`, `page` |

## Rules

- If it prints `no tpdf reader is open`, tell him. Don't guess from recent files.
- If `other_readers` isn't empty and `relation` is `elsewhere`, you may have the
  wrong one. Check that `path` fits the conversation, or ask. Pick another with
  `tpdf context --pid <pid> --png <file>`.
- A page can hold several theorems or questions. If "this" could mean more than
  one, ask which he means.
- This shows the PDF only. His work and handwriting are on the tablet.
