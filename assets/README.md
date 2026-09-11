# Brand assets

`logo.svg` is the editable source. The toolbar embeds `logo-216x88.rgba` so the
same image path works in native and WASM builds without another runtime decoder.

Regenerate the derivative with:

```sh
rsvg-convert --width 216 --height 88 logo.svg --output /tmp/funfern-logo.png
magick /tmp/funfern-logo.png -depth 8 rgba:logo-216x88.rgba
```
