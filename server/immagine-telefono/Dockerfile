# Telefono Android virtuale con i servizi Google: si parte dall'immagine redroid,
# che non li ha, e si aggiunge MindTheGapps (Android 13) dell'architettura della VM,
# arm64 o x86_64: Android gira nativo, senza emulazione. Con i servizi
# Google l'account Google si aggiunge nelle impostazioni come su un telefono vero
# e il sincronizzatore di Google copia i calendari in quello di sistema, dove
# l'agente li legge come quelli di Outlook.
# Release ufficiali di MindTheGapps, impronta controllata dalla build; le app
# dentro hanno la firma di Google. TARGETARCH lo dà BuildKit: si costruisce solo
# lo stage della VM.
FROM alpine:3.20 AS gapps-arm64
ADD --checksum=sha256:3ff2edaa75406eb4214402fca6906d4b413ab50afdb1af033d9e396992338d0b \
    https://github.com/MindTheGapps/13.0.0-arm64/releases/download/MindTheGapps-13.0.0-arm64-20231025_200931/MindTheGapps-13.0.0-arm64-20231025_200931.zip /gapps.zip

FROM alpine:3.20 AS gapps-amd64
ADD --checksum=sha256:2076179bcb6f30e78853d52cf70a4bd1d27502c3852332195e5356816cddfdd9 \
    https://github.com/MindTheGapps/13.0.0-x86_64/releases/download/MindTheGapps-13.0.0-x86_64-20231025_201203/MindTheGapps-13.0.0-x86_64-20231025_201203.zip /gapps.zip

FROM gapps-${TARGETARCH} AS gapps
RUN unzip -q /gapps.zip -d /gapps && chmod -R u+rwX,go+rX /gapps/system

FROM redroid/redroid:13.0.0_64only-latest
COPY --from=gapps /gapps/system /system
# senza ro.setupwizard.mode la procedura guidata di Google si apre al primo avvio
ENTRYPOINT ["/init", "androidboot.hardware=redroid", "ro.setupwizard.mode=DISABLED"]
