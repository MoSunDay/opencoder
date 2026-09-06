import { Button, Select, Space } from 'antd';
import { useEffect, useState } from 'react';
import { downloadArtifact, prepareArtifactDownloads } from './download.js';
export function Artifacts({ id, spec, onNotice }) {
  const [step, setStep] = useState(null); const [file, setFile] = useState('output.txt'); const [busy, setBusy] = useState(false);
  useEffect(() => { prepareArtifactDownloads().catch(() => {}); }, []);
  const download = async () => {
    setBusy(true);
    try {
      await downloadArtifact(id, step, file);
    } catch (e) { onNotice(e.message); }
    finally { setBusy(false); }
  };
  return <Space style={{ marginTop: 16 }}>
    <Select placeholder="选择步骤" value={step} onChange={setStep} style={{ width: 180 }} options={(spec?.steps || []).map((s) => ({ value: s.name, label: s.name }))} />
    <Select value={file} onChange={setFile} style={{ width: 160 }} options={['output.txt', 'output.json', 'meta.json'].map((value) => ({ value, label: value }))} />
    <Button disabled={!step} loading={busy} onClick={download}>下载节点产物</Button>
  </Space>;
}
