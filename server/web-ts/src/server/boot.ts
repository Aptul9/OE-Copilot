import { getMigrations } from "better-auth/db/migration";
import { auth, authOptions } from "@/lib/auth";
import { config } from "@/lib/config";
import { archivio } from "@/lib/core/archivio";
import { appDb } from "@/lib/db";
import { migrateLockout } from "@/lib/lockout";
import { syncOwner } from "@/lib/owner";

function fatal(message: string): never {
  console.error(message);
  process.exit(1);
}

export async function boot() {
  if (config.authSecret.length < 32) fatal("BETTER_AUTH_SECRET deve essere impostata, almeno 32 caratteri (openssl rand -hex 32)");
  // prima le tabelle: better-auth controlla lo schema quando si crea l'istanza
  const { runMigrations } = await getMigrations(authOptions());
  await runMigrations();
  migrateLockout(appDb());
  try {
    // lo stato dell'app: la prima volta importa config.json e local_events.json; se non riesce si riprova alla prima richiesta
    archivio();
  } catch (e) {
    console.error("archivio: stato dell'app non pronto", e);
  }
  try {
    console.log(await syncOwner(auth(), appDb()));
  } catch (e) {
    fatal(e instanceof Error ? e.message : String(e));
  }
}
