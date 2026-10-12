"use client";

import { type ReactNode, useState } from "react";

/**
 * Un logo che, se non si carica (il file non c'è più, il server non risponde), lascia il posto all'alternativa, di norma le iniziali: mai
 * l'icona dell'immagine rotta. Chi lo usa mette `key={src}`: un indirizzo nuovo riprova.
 */
export function ImgLogo({ src, className, alternativa }: { src: string; className?: string; alternativa: ReactNode }) {
  const [rotto, setRotto] = useState(false);
  if (rotto) return <>{alternativa}</>;
  // eslint-disable-next-line @next/next/no-img-element
  return <img src={src} alt="" className={className} onError={() => setRotto(true)} />;
}
