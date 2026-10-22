"""Genera le icone di OE Copilot. Rilanciare solo per ritoccarle.

Il disegno: tre calendari sovrapposti e un po' sfasati, ognuno del suo colore (come le
aziende nella barra laterale). Si disegna in grande e si riduce con LANCZOS per bordi netti.

Scrive app/app-icon.png (l'icona di partenza per ogni altro formato, quella che `tauri icon`
legge senza argomenti) e, se c'è la cartella dell'interfaccia nel browser, anche
server/web-ts/src/app/icon.png e apple-icon.png (sfondo pieno: iOS riempie di nero il trasparente).
La sagoma bianca su trasparente, l'unica che la barra di stato di Android sa mostrare, va in
server/web-ts/public/badge.png (notifiche del browser) e nelle drawable-*/ic_stat_promemoria.png
dell'app Android (i promemoria).
"""
import os

from PIL import Image, ImageChops, ImageDraw, ImageFilter

# misure di lavoro: un calendario e quanto sporgono gli anelli
L, A, RAGGIO, BORDO, TESTATA = 1120, 1200, 130, 18, 300
# colore, rotazione in gradi (antioraria), centro: dal calendario dietro a quello davanti
CALENDARI = [
    ((99, 102, 241), 7, (1000, 1000)),     # indaco, il colore dell'app
    ((16, 185, 129), 1, (1215, 1190)),     # verde
    ((249, 115, 22), -6, (1430, 1385)),    # arancio
]


def calendario(colore):
    """Un calendario (testata colorata, due anelli, foglio bianco coi giorni) su fondo trasparente."""
    pad = 200
    img = Image.new("RGBA", (L + 2 * pad, A + 2 * pad), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    x0, y0 = pad, pad
    scuro = tuple(int(c * 0.6) for c in colore)
    chiaro = tuple(round(c + (255 - c) * 0.8) for c in colore)
    # tutto del colore (bordo e testata), poi il foglio bianco che lascia scoperta la testata
    d.rounded_rectangle([x0, y0, x0 + L, y0 + A], RAGGIO, fill=colore)
    d.rounded_rectangle([x0 + BORDO, y0 + TESTATA, x0 + L - BORDO, y0 + A - BORDO], RAGGIO - BORDO,
                        fill=(255, 255, 255), corners=(False, False, True, True))
    for fx in (0.27, 0.73):
        cx = x0 + L * fx
        d.rounded_rectangle([cx - 36, y0 - 80, cx + 36, y0 + 130], 36, fill=scuro)
    # tre colonne e tre righe di giorni; quello al centro è "oggi"
    gap, margine = 56, 100
    larg = (L * 0.74 - 2 * gap) / 3
    alt = (A - TESTATA - 2 * margine - 2 * gap) / 3
    for riga in range(3):
        for col in range(3):
            x = x0 + L * 0.13 + col * (larg + gap)
            y = y0 + TESTATA + margine + riga * (alt + gap)
            d.rounded_rectangle([x, y, x + larg, y + alt], 44, fill=colore if (riga, col) == (1, 1) else chiaro)
    return img


def composizione(ombre=True):
    """I tre calendari uno sull'altro, ognuno con la sua ombra, ritagliati e centrati in un quadrato."""
    tela = Image.new("RGBA", (2900, 2900), (0, 0, 0, 0))
    for colore, angolo, (cx, cy) in CALENDARI:
        c = calendario(colore).rotate(angolo, resample=Image.BICUBIC, expand=True)
        x, y = round(cx - c.width / 2), round(cy - c.height / 2)
        if ombre:
            ombra = Image.new("RGBA", c.size, (0, 0, 0, 0))
            ombra.putalpha(c.getchannel("A").point(lambda v: v * 0.38))
            ombra = ombra.filter(ImageFilter.GaussianBlur(28))
            tela.alpha_composite(ombra, (x - 14, y + 22))
        tela.alpha_composite(c, (x, y))
    tela = tela.crop(tela.getbbox())
    lato = round(max(tela.size) * 1.1)
    quadro = Image.new("RGBA", (lato, lato), (0, 0, 0, 0))
    quadro.alpha_composite(tela, ((lato - tela.width) // 2, (lato - tela.height) // 2))
    return quadro.resize((1024, 1024), Image.LANCZOS)


def su_fondo(icona, lato, fondo=(248, 248, 252), scala=0.78):
    """L'icona più piccola, al centro di un quadrato pieno (per apple-icon)."""
    quadro = Image.new("RGBA", (lato, lato), fondo + (255,))
    piccola = icona.resize((round(lato * scala),) * 2, Image.LANCZOS)
    quadro.alpha_composite(piccola, ((lato - piccola.width) // 2, (lato - piccola.height) // 2))
    return quadro.convert("RGB")


def sagoma(lato):
    """L'icona senza ombre in bianco su trasparente: pieno il colore, vuoto il foglio bianco e i giorni chiari (Android legge solo l'alfa)."""
    img = composizione(ombre=False)
    r, g, b, a = img.split()
    pieno = ImageChops.darker(ImageChops.darker(r, g), b).point(lambda v: 255 if v < 170 else 0)
    bianca = Image.new("RGBA", img.size, (255, 255, 255, 0))
    bianca.putalpha(ImageChops.multiply(pieno, a))
    return bianca.resize((lato, lato), Image.LANCZOS)


# la barra di stato di Android: 24 dp per densità
DENSITA = {"mdpi": 24, "hdpi": 36, "xhdpi": 48, "xxhdpi": 72, "xxxhdpi": 96}

if __name__ == "__main__":
    app = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    grande = composizione()
    icona = grande.resize((256, 256), Image.LANCZOS)
    icona.save(os.path.join(app, "app-icon.png"))
    scritti = ["app/app-icon.png"]
    web = os.path.join(os.path.dirname(app), "server", "web-ts", "src", "app")
    if os.path.isdir(web):
        grande.resize((512, 512), Image.LANCZOS).save(os.path.join(web, "icon.png"))
        su_fondo(grande, 180).save(os.path.join(web, "apple-icon.png"))
        sagoma(96).save(os.path.join(os.path.dirname(os.path.dirname(web)), "public", "badge.png"))
        scritti += ["server/web-ts/src/app/icon.png", "server/web-ts/src/app/apple-icon.png", "server/web-ts/public/badge.png"]
    res = os.path.join(app, "src-tauri", "gen", "android", "app", "src", "main", "res")
    for densita, lato in DENSITA.items():
        os.makedirs(os.path.join(res, f"drawable-{densita}"), exist_ok=True)
        sagoma(lato).save(os.path.join(res, f"drawable-{densita}", "ic_stat_promemoria.png"))
    scritti.append("drawable-*/ic_stat_promemoria.png")
    print("rigenerati:", ", ".join(scritti))
