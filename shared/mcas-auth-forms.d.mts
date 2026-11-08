export function configuration(source: string, marker: string): Record<string, unknown>;
export function mcasForm(html: string): { action: string; idToken: string };
export function mcasContinuation(html: string): { action: string; fields: Record<string, string> };
export function mcasRestore(value: string, gateway: string): { action: string; fields: Record<string, string> };
export function bootstrapUrl(html: string, origin: string): string | null;
export function mcasMoved(html: string, appOrigin?: string): string;
