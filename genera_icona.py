"""Genera icon.ico / icon.png per OE Copilot. Rilanciare solo per ritoccarla.

Disegnato in grande e ridotto con LANCZOS per bordi netti. Motivo: quadrato
viola sfumato (come l'app), con le lettere "OE" bianche in evidenza e un piccolo
segno di calendario/spunta, a richiamare "OverEmployed" + agenda.
"""
import os

from PIL import Image, ImageDraw, ImageFont, ImageFilter

VIOLA_ALTO = (124, 130, 240)
VIOLA_BASSO = (72, 76, 168)
BIANCO = (255, 255, 255)
LATO = 1024


def sfumatura(dim, alto, basso):
    grad = Image.new("RGB", (1, dim))
    for y in range(dim):
        t = y / (dim - 1)
        grad.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(alto, basso)))
    return grad.resize((dim, dim))


def _font(dimensione):
    for nome in ("segoeuib.ttf", "arialbd.ttf", "seguisb.ttf", "Arial_Bold.ttf"):
        try:
            return ImageFont.truetype(nome, dimensione)
        except OSError:
            continue
    return ImageFont.load_default()


# Sfondo: quadrato arrotondato con sfumatura viola.
maschera = Image.new("L", (LATO, LATO), 0)
ImageDraw.Draw(maschera).rounded_rectangle([36, 36, LATO - 36, LATO - 36], radius=228, fill=255)
img = Image.new("RGBA", (LATO, LATO), (0, 0, 0, 0))
img.paste(sfumatura(LATO, VIOLA_ALTO, VIOLA_BASSO).convert("RGBA"), (0, 0), maschera)

d = ImageDraw.Draw(img)

# Lettere "OE" centrate, in grassetto bianco.
font = _font(430)
testo = "OE"
box = d.textbbox((0, 0), testo, font=font)
w, h = box[2] - box[0], box[3] - box[1]
x = (LATO - w) / 2 - box[0]
y = (LATO - h) / 2 - box[1] - 28
d.text((x, y), testo, font=font, fill=BIANCO)

# Piccola spunta/segno agenda in basso, come accento.
d.rounded_rectangle([past := 372, 792, LATO - 372, 792 + 34], radius=17, fill=(255, 255, 255, 205))

CARTELLA = os.path.dirname(os.path.abspath(__file__))
misure = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
grande = img.resize((256, 256), Image.LANCZOS)
grande.save(os.path.join(CARTELLA, "icon.ico"), format="ICO", sizes=misure)
grande.save(os.path.join(CARTELLA, "icon.png"))
print("icon.ico e icon.png rigenerati in", CARTELLA)

