"""Derive Android assets from the existing desktop Riviu mark; requires Pillow."""
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT.parents[1] / "apps/desktop/public/logo.jpg"
RES = ROOT / "app/src/main/res"
DENSITIES = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}


def logo() -> Image.Image:
    original = Image.open(SOURCE).convert("RGB")
    # The supplied JPEG is the orange R on white. Extract its existing silhouette;
    # use its brand orange and keep antialias coverage while discarding JPEG noise.
    alpha = Image.new("L", original.size)
    alpha.putdata([max(0, 255 - b) if r > 150 and r - g > 45 and g - b > 20 else 0
                   for r, g, b in original.get_flattened_data()])
    mark = Image.new("RGBA", original.size, (255, 102, 0, 0))
    mark.putalpha(alpha)
    return mark.crop(alpha.getbbox())


def centered(mark: Image.Image, canvas: int, glyph: int) -> Image.Image:
    result = Image.new("RGBA", (canvas, canvas))
    scaled = mark.copy()
    scaled.thumbnail((glyph, glyph), Image.Resampling.LANCZOS)
    result.alpha_composite(scaled, ((canvas - scaled.width) // 2, (canvas - scaled.height) // 2))
    return result


def save(image: Image.Image, relative: str) -> None:
    destination = RES / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    image.save(destination, optimize=True)


def main() -> None:
    mark = logo()
    white = Image.new("RGBA", mark.size, "white")
    white.putalpha(mark.getchannel("A"))
    save(centered(mark, 256, 248), "drawable-nodpi/riviu_logo.png")
    for density, scale in DENSITIES.items():
        size = round(48 * scale)
        for name, rounded in (("ic_launcher", False), ("ic_launcher_round", True)):
            icon = Image.new("RGBA", (size, size))
            draw = ImageDraw.Draw(icon)
            if rounded:
                draw.ellipse((0, 0, size - 1, size - 1), fill="white")
            else:
                draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=round(size * 0.2), fill="white")
            icon.alpha_composite(centered(mark, size, round(size * 0.66)))
            save(icon, f"mipmap-{density}/{name}.png")
        save(centered(mark, round(108 * scale), round(54 * scale)),
             f"mipmap-{density}/ic_launcher_foreground.png")
        save(centered(white, round(108 * scale), round(54 * scale)),
             f"mipmap-{density}/ic_launcher_monochrome.png")
        save(centered(white, round(24 * scale), round(22 * scale)),
             f"drawable-{density}/ic_notification.png")
    print("Generated Riviu launcher, adaptive, monochrome and notification assets from apps/desktop/public/logo.jpg")


if __name__ == "__main__":
    main()
