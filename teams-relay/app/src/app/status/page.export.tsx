import { StatusExported } from "@/components/ExportedPages";

// every status slug (src/app/[slug]/page.tsx): the Rust web app sends this page with the answer of the slug and its title
export default function Status() {
  return <StatusExported />;
}
