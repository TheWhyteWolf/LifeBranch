# lifecursor

LifeBranch's cursor themes, drawn by code rather than shipped as images.
It writes two themes to `~/.local/share/icons`:

- **LifeBranch-dark:** a black cursor with a white ring and a thin black edge
- **LifeBranch-light:** the reverse

Either one stays visible on any background, light, dark or busy, because its
outline always contrasts with whatever is behind it. Pick one in
**Settings > Cursor** (`lifeconf --gui --panel cursor`).

```
lifecursor                     # both themes into ~/.local/share/icons
lifecursor --dir DIR           # somewhere else
lifecursor --sheet out.ppm     # also a contact sheet of every cursor, for checking
```

The installers build and run it. Running it again overwrites the themes in place.

## How it draws

Each cursor is a stack of shapes on a 32×32 grid (`src/cursors.rs`): polygons,
round-ended lines, circles, rounded boxes and arcs. Every pixel takes its
signed distance to those shapes (`src/sdf.rs`). That one number gives the fill,
the ring and the edge, each anti-aliased, at every size from 16 to 128 px.
Badges such as progress, help and copy are layers painted over the arrow, each
with its own outline. The ring and edge keep a minimum width in pixels, so
small cursors still show a clear outline.

`wait` and `progress` spin over 12 frames. The files are standard Xcursor
(`src/xcursor.rs`). Each cursor is one file, and every name an app might ask
for, CSS (`ew-resize`), X11 (`sb_h_double_arrow`) or Qt's hashes, is a
symlink to it.

## Why the outline doesn't invert what's behind it

Windows has cursors that invert the pixels beneath them. On Linux that isn't
possible in a cursor theme: an Xcursor image is plain ARGB, and niri, like
every Wayland compositor, draws it with ordinary alpha blending. A true
inverting cursor would need niri itself to draw the cursor with a difference
blend. A ring in one contrasting colour and an edge in the other gets the
same result on any background: the shape stays readable.

## Cost

It runs once at install and takes under a second to draw everything. It takes
about 9 MB of disk per theme, mostly the large sizes and the animation frames.
At runtime only the size in use is loaded, by niri and by each app, the same
as any other cursor theme.

## Testing

`cargo test` covers the distance functions (signs, arcs, rotation), the fill,
ring and edge order and premultiplied alpha, and the Xcursor table of
contents. It also checks that every cursor name is unique, the core CSS names
all exist, and each hotspot sits on its shape.
