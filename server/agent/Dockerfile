FROM python:3.12-slim
# tzdata: gli orari si convertono nel fuso di Roma come fa l'app sul PC.
RUN apt-get update && apt-get install -y --no-install-recommends tzdata \
    && rm -rf /var/lib/apt/lists/*
# Le librerie alle versioni in esercizio l'11/10/2026: la prova prova_agente_golden.py registra cosa ne esce, e una
# versione nuova che cambia le ricorrenze non deve entrare da sola a ogni costruzione dell'immagine.
COPY requirements.txt /tmp/requirements.txt
RUN pip install --no-cache-dir -r /tmp/requirements.txt
ENV TZ=Europe/Rome
WORKDIR /app
CMD ["python", "-u", "/app/agent.py"]
