# Telefono Android virtuale con i servizi Google: si parte dall'immagine redroid,
# che non li ha, e si aggiunge MindTheGapps (Android 13, arm64). Con i servizi
# Google l'account Google si aggiunge nelle impostazioni come su un telefono vero
# e il sincronizzatore di Google copia i calendari in quello di sistema, dove
# l'agente li legge come quelli di Outlook.
# Release ufficiale di MindTheGapps, impronta controllata dalla build; le app
# dentro hanno la firma di Google.
FROM alpine:3.20 AS gapps
ADD --checksum=sha256:3ff2edaa75406eb4214402fca6906d4b413ab50afdb1af033d9e396992338d0b \
    https://github.com/MindTheGapps/13.0.0-arm64/releases/download/MindTheGapps-13.0.0-arm64-20231025_200931/MindTheGapps-13.0.0-arm64-20231025_200931.zip /gapps.zip
RUN unzip -q /gapps.zip -d /gapps && chmod -R u+rwX,go+rX /gapps/system

FROM redroid/redroid:13.0.0_64only-latest
COPY --from=gapps /gapps/system /system
# senza ro.setupwizard.mode la procedura guidata di Google si apre al primo avvio
ENTRYPOINT ["/init", "androidboot.hardware=redroid", "ro.setupwizard.mode=DISABLED"]
