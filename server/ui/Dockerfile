FROM python:3.12-slim
# tzdata: gli orari si convertono nel fuso di Roma come fa l'app sul PC.
RUN apt-get update && apt-get install -y --no-install-recommends tzdata \
    && rm -rf /var/lib/apt/lists/*
# Le stesse librerie dell'agente e dell'app sul PC.
RUN pip install --no-cache-dir requests icalendar recurring-ical-events
ENV TZ=Europe/Rome OE_DATI=/dati
# Senza privilegi: /dati (configurazione, cache, eventi personali) è l'unica
# cartella scrivibile e sul server è montata da fuori.
RUN useradd --uid 10001 --no-create-home oe && mkdir /dati && chown oe /dati
WORKDIR /app
# app/ = moduli e pagina dell'app del PC, messi qui da strumenti/prepara_ui.sh
COPY app/ /app/
COPY server.py /app/
USER oe
CMD ["python", "-u", "/app/server.py"]
