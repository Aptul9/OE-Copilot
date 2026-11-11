//! L'accesso: la parte di Better Auth 1.7 che la pagina usa (era lib/auth.ts con better-auth, il plugin two-factor e @better-auth/passkey),
//! sullo stesso gateway.db e con gli stessi cookie, così le sessioni, i TOTP e le passkey di prima restano validi.

pub mod cbor;
pub mod cookie;
pub mod cripto;
pub mod due_fattori;
pub mod limiti;
pub mod passkey;
pub mod righe;
pub mod rotte;
pub mod webauthn;
