export const MAX_BINARY_BYTES = 32 * 1024 * 1024;

export function parseResource(value) {
  const matched = /^([A-Za-z0-9_][A-Za-z0-9_.-]{0,47})(?:@v([1-9][0-9]*))?$/.exec(value || '');
  const version = matched?.[2] ? Number(matched[2]) : null;
  return matched && (!version || version <= 4294967295) ? { name: matched[1], version } : null;
}

export function resourceToken(name, version) {
  return version ? `${name}@v${version}` : name;
}

export function versionOptions(pool) {
  return [
    { value: 0, label: `当前版本${pool?.current ? ` v${pool.current}` : ''}（受理时固定）` },
    ...(pool?.history || []).filter(Boolean).map((version) => ({ value: version.version, label: `固定 v${version.version}` })),
  ];
}

export function validateUpload(bytes) {
  if (!bytes.byteLength || bytes.byteLength > MAX_BINARY_BYTES) throw new Error('请选择不超过 32 MiB 的 Linux ELF 可执行文件');
  if (bytes.byteLength < 64 || bytes[0] !== 127 || bytes[1] !== 69 || bytes[2] !== 76 || bytes[3] !== 70) throw new Error('文件不是 Linux ELF 可执行文件');
  if (bytes[4] !== 2 || bytes[5] !== 1) throw new Error('仅支持小端 ELF64 可执行文件');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (![2, 3].includes(view.getUint16(16, true)) || ![62, 183].includes(view.getUint16(18, true))) throw new Error('仅支持 x86_64 或 aarch64 Linux 可执行文件');
  return bytes;
}

export function encodeUpload(buffer) {
  const bytes = validateUpload(new Uint8Array(buffer));
  const parts = [];
  for (let offset = 0; offset < bytes.length; offset += 8192) parts.push(String.fromCharCode(...bytes.subarray(offset, offset + 8192)));
  return btoa(parts.join(''));
}
