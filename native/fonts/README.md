Inter Variable is the unmodified TrueType file from Google Fonts commit `e1d6480102fed30739fead0faee463101f892c8f`. Its font version `4.001;git-66647c0bb` matches the bundled Electron web font. [Source](https://github.com/google/fonts/tree/e1d6480102fed30739fead0faee463101f892c8f/ofl/inter)

Windows and Linux load `Refract-Inter.ttc`. It contains the exact weights used by the Electron stylesheet, with optical size fixed to the source font's default of 14, as in the bundled `wght` web font. GPUI 0.2.2 on Linux selects a static face by weight, so it needs separate faces to render those weights. The collection uses its own family name to avoid a collision with system-installed Inter. macOS uses its system UI font.

To regenerate the collection, run this Python snippet from this directory with FontTools 4.66.1 installed.

```python
from fontTools.ttLib import TTCollection, TTFont
from fontTools.varLib.instancer import instantiateVariableFont

family = "Refract Inter"
styles = {
    400: "Regular",
    500: "Medium",
    520: "Weight 520",
    550: "Weight 550",
    580: "Weight 580",
    600: "SemiBold",
    620: "Weight 620",
    650: "Weight 650",
    700: "Bold",
}
collection = TTCollection()
for weight, style in styles.items():
    font = TTFont("Inter-Variable.ttf", recalcTimestamp=False)
    instantiateVariableFont(font, {"opsz": 14, "wght": weight}, inplace=True)
    postscript = f"RefractInter-{style.replace(' ', '')}"
    for name_id, value in {
        1: family,
        2: style,
        3: f"{font['name'].getDebugName(5)};{postscript}",
        4: f"{family} {style}",
        6: postscript,
        16: family,
        17: style,
    }.items():
        font["name"].setName(value, name_id, 3, 1, 0x0409)

    font["OS/2"].fsSelection &= ~0x60
    font["head"].macStyle &= ~1
    if weight == 400:
        font["OS/2"].fsSelection |= 0x40
    if weight == 700:
        font["OS/2"].fsSelection |= 0x20
        font["head"].macStyle |= 1
    collection.fonts.append(font)

collection.save("Refract-Inter.ttc")
```

Both files are available under the [SIL Open Font License](../../resources/fonts/Inter-OFL.txt).
