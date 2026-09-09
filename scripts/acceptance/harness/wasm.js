const fs = require('node:fs');
const path = require('node:path');

// The embedded Wasmtime runtime accepts WAT as well as encoded wasm modules.
function stageWasm(dataDir, message = 'node artifact') {
  const directory = path.join(dataDir, 'dag/_modules');
  fs.mkdirSync(directory, { recursive: true });
  const data = Buffer.from(`${message}\n`);
  const escaped = [...data].map(byte => `\\${byte.toString(16).padStart(2, '0')}`).join('');
  fs.writeFileSync(path.join(directory, 'stdout.wasm'), `(module
    (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
    (memory (export "memory") 1) (data (i32.const 0) "${escaped}")
    (func (export "_start") (i32.store (i32.const 1024) (i32.const 0))
      (i32.store (i32.const 1028) (i32.const ${data.length}))
      (drop (call $write (i32.const 1) (i32.const 1024) (i32.const 1) (i32.const 1032)))))`);
  fs.writeFileSync(path.join(directory, 'spin.wasm'), '(module (func (export "_start") (loop $l (br $l))))');
}
module.exports = { stageWasm };
