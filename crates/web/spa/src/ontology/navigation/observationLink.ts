export type ObservationLink = { aspectKey?: string; centerId?: string; upstreamDepth?: number; downstreamDepth?: number };
const fields = ["ontology_aspect", "ontology_center", "ontology_upstream", "ontology_downstream"];

export function observationLink(search: string, hash = ""): ObservationLink | null {
  for (const value of [hash, search]) {
    const params = new URLSearchParams(value.replace(/^[?#]/, ""));
    const aspectKey = params.get(fields[0])?.trim() || undefined;
    const centerId = params.get(fields[1])?.trim() || undefined;
    if (!aspectKey && !centerId) continue;
    const depth = (key: string) => {
      const raw = params.get(key);
      return raw !== null && /^[0-9]$/.test(raw) ? Number(raw) : undefined;
    };
    return { aspectKey, centerId, upstreamDepth: depth(fields[2]), downstreamDepth: depth(fields[3]) };
  }
  return null;
}

export function withoutObservationLink(href: string) {
  const url = new URL(href);
  const hash = new URLSearchParams(url.hash.slice(1));
  const hasFragment = fields.some((field) => hash.has(field));
  for (const field of fields) { url.searchParams.delete(field); hash.delete(field); }
  if (hasFragment) url.hash = hash.toString();
  return url.pathname + url.search + url.hash;
}
