import { redirect } from "next/navigation";

// Era l'indirizzo provvisorio della nuova interfaccia: ora è la pagina principale.
export default function Nuova() {
  redirect("/");
}
