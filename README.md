**tpdf**  
a pdf reader for the terminal. fast, small, vim keys, and it draws inside tmux panes.

## install

needs ghostty or kitty, plus homebrew's mupdf.

```sh
brew install mupdf
cargo install --path .
```

`tpdf <file.pdf> [page]`

each file reopens where you left it, at the same zoom. pass a page to open there instead. dark mode is remembered across all files.

rebuild after `brew upgrade mupdf`. tpdf tells you when it's needed.

## keys

| key | action |
| --- | --- |
| `j` `k` `h` `l` | scroll |
| `ctrl-d` `ctrl-u` | half a screen |
| `ctrl-f` `ctrl-b` `space` | a full screen |
| `J` `K` | next / previous page |
| `gg` `G` | first page / end |
| `12G` `12gg` `:12` | page 12 |
| `ctrl-o` `ctrl-i` | back / forward through places you jumped from |
| `=` `-` | zoom |
| `+` `_` | zoom faster |
| `:150%` | set zoom, relative to fit-page |
| `0` `w` | fit page / toggle fit width |
| `/` `?` | search forward / back |
| `n` `N` | next / previous match |
| `esc` | clear search |
| `z` | dark mode |
| `f` | toggle status bar |
| `r` | reload |
| `ctrl-l` | redraw |
| `q` `:q` | quit |

counts work on motions, like `5j` or `3J`. the file reloads itself when it changes.

## agents

`tpdf context --png view.png` prints JSON describing the open reader: the file, page, visible region, and text. it also writes a picture of what's on screen. with several readers open it picks the one nearest the caller: same tmux window, then same session, then on screen, then most recently moved. use `--pid` to pick one yourself.

readers publish to `~/.local/state/tpdf/readers/<pid>.json` when the view settles and remove the file on quit. the [`tpdf-reading`](skills/tpdf-reading/SKILL.md) skill tells agents how to use this.

## how it stays fast in tmux

- pages are drawn with [unicode placeholders](https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders), so tmux sees ordinary text cells and handles positioning, splits, and window switches.
- mupdf renders straight into shared memory and the terminal reads it from there, so page data never passes through tmux.
- pages are split into tiles. only what's on screen gets rendered, plus one screen either way when idle.
- key repeats collapse into one redraw, and redraws are capped at 60 per second, so holding `j` can't build up lag.
- tmux drops output, including images, when a client falls behind. tpdf keeps that from happening while it scrolls, and re-sends its images if another pane causes it anyway.
- images in the terminal are capped at 64 MB and mupdf's cache at 32 MB.

tpdf sets `allow-passthrough all` on its own pane, so tmux needs no config.

## limits

- local terminals only: shared memory doesn't cross ssh.
- no nested tmux.
- no password-protected files.

`tpdf --bench <file.pdf>` prints open, render, and memory numbers.
