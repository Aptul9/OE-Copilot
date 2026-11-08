import { rename, writeFile } from "node:fs/promises";

/** Scrittura atomica (file provvisorio accanto, poi rename): chi legge non vede mai un file a metà. */
export async function scriviAtomico(percorso: string, testo: string): Promise<void> {
  const provvisorio = percorso + ".tmp";
  await writeFile(provvisorio, testo);
  await rename(provvisorio, percorso);
}
