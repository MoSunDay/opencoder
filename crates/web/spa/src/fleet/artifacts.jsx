import { Button, Input, InputNumber, Select, Space } from 'antd';
import { useEffect, useState } from 'react';
import { downloadArtifact, prepareArtifactDownloads } from './download.js';
import { err } from '../notice.js';
export function Artifacts({ id, spec, onNotice }) {
  const [index, setIndex] = useState(null);
  const [step, setStep] = useState(null); const [file, setFile] = useState('output.txt'); const [busy, setBusy] = useState(false);
  useEffect(() => { prepareArtifactDownloads().catch(() => {}); }, []);
  const download = async () => {
    setBusy(true);
    try {
      await downloadArtifact(id, step, file, index);
    } catch (e) { onNotice(err(e.message)); }
    finally { setBusy(false); }
  };
  return <Space wrap style={{ marginTop: 16 }}>
    <Select placeholder="选择步骤" value={step} onChange={setStep} style={{ width: 180 }} options={(spec?.steps || []).map((s) => ({ value: s.name, label: s.name }))} />
    <InputNumber aria-label="动态实例索引" placeholder="实例索引（从 0 开始）" min={0} precision={0} value={index} onChange={setIndex} style={{ width: 180 }} />
    <Input aria-label="产物文件路径" placeholder="文件路径，如 report.zip" value={file} onChange={(e) => setFile(e.target.value)} style={{ width: 260 }} />
    <Button disabled={!step || !file.trim()} loading={busy} onClick={download}>下载节点产物</Button>
  </Space>;
}
