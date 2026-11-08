export { espandiRigheAndroid, durataAndroid, type OccorrenzaAndroid, type OpzioniAndroid, type RigaAndroid } from "./android.ts";
export { espandiIcs, validaIcs, leggiIsoLocale, MEETING_URL, type EventoIcs, type OpzioniIcs } from "./ics.ts";
export { ErroreAltro, ErroreSenzaRisposta, ErroreTipo, ErroreValore } from "./tempo.ts";
export { ErroreNonSupportato } from "./regola.ts";
export { CalendarioAndroid, DB, estrai, eventiDaLettura, type Contesto, type Evento, type FonteCalendario, type Lettura, type RigaDb } from "./fonte.ts";
export {
  CalendarioDiretto,
  ErroreAccessoScaduto,
  GOOGLE_CALENDAR_APP,
  GOOGLE_CALENDAR_SERVICE,
  GOOGLE_CLIENT_SIG,
  GestoreToken,
  OUTLOOK_CLIENT_ID,
  OUTLOOK_REDIRECT_URI,
  OUTLOOK_SCOPES,
  type ArchivioToken,
  type TokenPostaInfo,
  type VoceToken,
} from "./diretto.ts";
export {
  OUTLOOK_WEB_CLIENT_ID,
  OUTLOOK_WEB_REDIRECT_URI,
  OUTLOOK_WEB_SCOPES,
  SessioneScaduta,
  autorizzaDaSessione,
  leggiSessione,
  type CookieSessione,
  type SessioneMicrosoft,
} from "./sessione-web.ts";
export {
  MS_LOGIN,
  OUTLOOK_REST,
  ErroreRispostaGrande,
  claimsJwt,
  getRest,
  jsonRest,
  motivoTokenWebNonValido,
  upnDi,
  type FetchFn,
  type FonteToken,
  type OpzioniRest,
  type RispostaRest,
} from "./outlook-rest.ts";
export { ErroreComunicazione, GatewayComunicazione, MAX_PERCORSO, MAX_RISPOSTA, percorsoLettura, preferFuso, type CasellaComunicazione } from "./comunicazione.ts";
export { Agente, caricaFonti, contestoDi, creaServer, registroSuStderr, type OpzioniAgente, type Registro } from "./server.ts";
export { htmlUnescape, pulisciLink } from "./html.ts";
export { fromIsoformat, isoNaive, oraLocaleDa } from "./iso.ts";
export { analizzaTarget, parseQs } from "./query.ts";
export { confrontaPy, jsonPy, pyRepr, pyReprStr, pyStrip } from "./pyfmt.ts";
export { GoogleCommunication } from "./google-communication.ts";
export { GmailGateway, GmailError, gmailReadPath } from "./gmail.ts";
export { GoogleChat, GoogleChatError, type Conversation, type ChatMessage } from "./gchat.ts";
