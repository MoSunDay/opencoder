/** RFC 4122 version 4 UUID; getRandomValues also works on HTTP inspection endpoints. */
export function requestId(bytes: Uint8Array): string {
  if (bytes.length !== 16) throw new Error("UUID requires 16 random bytes");
  const value = Array.from(bytes);
  value[6] = (value[6] & 0x0f) | 0x40;
  value[8] = (value[8] & 0x3f) | 0x80;
  const hex = value.map((byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
