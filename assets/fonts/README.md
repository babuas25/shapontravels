# Ticket PDF font

Noto Sans Regular by The Noto Project Authors, licensed under the accompanying SIL OFL.
Source: https://github.com/notofonts/noto-fonts/blob/main/hinted/ttf/NotoSans/NotoSans-Regular.ttf

`NotoSans-Regular-Print.ttf` retains every source glyph but removes screen hinting to
reduce the size of email attachments. It was generated with FontTools:

```sh
python -m fontTools.subset NotoSans-Regular.ttf --glyphs='*' --no-hinting --output-file=NotoSans-Regular-Print.ttf
```
No runtime font downloads or FontTools dependency are required.
