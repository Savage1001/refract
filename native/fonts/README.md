Inter Variable is the unmodified TrueType file from Google Fonts commit `e1d6480102fed30739fead0faee463101f892c8f`. Its font version `4.001;git-66647c0bb` matches the bundled Electron web font. [Source](https://github.com/google/fonts/tree/e1d6480102fed30739fead0faee463101f892c8f/ofl/inter)

Windows and Linux load `Refract-Inter.ttc`. It contains the exact weights used by the Electron stylesheet, with optical size fixed to the source font's default of 14, as in the bundled `wght` web font. GPUI 0.2.2 on Linux selects a static face by weight, so it needs separate faces to render those weights. The collection uses its own family name to avoid a collision with system-installed Inter. macOS uses its system UI font.

The Regular Italic and Bold Italic faces shear the corresponding upright outlines by one quarter, matching Chromium's synthetic italic for the upright-only Inter web font. The italic faces keep the upright advances. [Chromium's synthetic italic](https://chromium.googlesource.com/chromium/src/+/8075357d430a75b0f068431f7153bf5df64036aa/third_party/blink/renderer/platform/fonts/font_platform_data.cc)

To regenerate the collection, run this Python snippet from this directory with FontTools 4.66.1 installed.

```python
from math import atan, degrees

from fontTools.misc.roundTools import otRound
from fontTools.ttLib import TTCollection, TTFont
from fontTools.ttLib.tables._g_l_y_f import GlyphCoordinates
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


def slant(font):
    outlines = font["glyf"]
    original_bounds = {}
    for name in font.getGlyphOrder():
        glyph = outlines[name]
        if not glyph.numberOfContours:
            continue
        original_bounds[name] = glyph.xMin
        if glyph.isComposite():
            for component in glyph.components:
                # Inter's composites only translate the component outlines.
                if hasattr(component, "transform") or not hasattr(component, "x"):
                    raise ValueError(f"Unexpected component transform in {name}")
                component.x = otRound(component.x + component.y / 4)
        else:
            glyph.coordinates = GlyphCoordinates(
                (otRound(x + y / 4), y) for x, y in glyph.coordinates
            )

    for name, original_minimum in original_bounds.items():
        glyph = outlines[name]
        glyph.recalcBounds(outlines)
        advance, bearing = font["hmtx"][name]
        font["hmtx"][name] = (advance, bearing + glyph.xMin - original_minimum)

    font["post"].italicAngle = -degrees(atan(0.25))
    # These static faces use the italic and weight flags below to advertise
    # style. The source STAT table describes the upright variation axes.
    del font["STAT"]


def face(weight, style, italic=False):
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
    if weight == 400 and not italic:
        font["OS/2"].fsSelection |= 0x40
    if weight == 700:
        font["OS/2"].fsSelection |= 0x20
        font["head"].macStyle |= 1
    if italic:
        slant(font)
        font["OS/2"].fsSelection |= 1
        font["head"].macStyle |= 2
    return font


collection = TTCollection()
collection.fonts.extend(face(weight, style) for weight, style in styles.items())
collection.fonts.append(face(400, "Italic", italic=True))
collection.fonts.append(face(700, "Bold Italic", italic=True))
collection.save("Refract-Inter.ttc")
```

Both files are available under the [SIL Open Font License](../../resources/fonts/Inter-OFL.txt).
