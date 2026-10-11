import { passkeyClient } from "@better-auth/passkey/client";
import { twoFactorClient } from "better-auth/client/plugins";
import { createAuthClient } from "better-auth/react";

// Stessa origine della pagina: niente baseURL. I plugin del secondo fattore ci sono sempre; le strade rispondono 404 finché AUTH_2FA è off
export const authClient = createAuthClient({ plugins: [twoFactorClient(), passkeyClient()] });
