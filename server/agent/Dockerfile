FROM python:3.12-slim
# tzdata: gli orari si convertono nel fuso di Roma come fa l'app sul PC.
RUN apt-get update && apt-get install -y --no-install-recommends tzdata \
    && rm -rf /var/lib/apt/lists/*
# Le stesse librerie dell'app sul PC, per leggere i calendari allo stesso modo.
RUN pip install --no-cache-dir requests icalendar recurring-ical-events
ENV TZ=Europe/Rome
WORKDIR /app
CMD ["python", "-u", "/app/agent.py"]
