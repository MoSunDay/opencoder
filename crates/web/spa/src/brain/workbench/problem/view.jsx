import { Alert, Image, Space, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../../api.js';

export function ProblemView({ problem }) {
  const [images, setImages] = useState([]); const [error, setError] = useState('');
  const imageReferences = JSON.stringify(problem?.images || []);
  useEffect(() => {
    let alive = true; setImages([]); setError('');
    Promise.all(JSON.parse(imageReferences).map(async (ref) => {
      const object = await apiGet(`/api/brain/attachments/${encodeURIComponent(ref.id)}`);
      if (object.reference.sha256 !== ref.sha256) throw new Error('问题图片校验失败');
      return { ...ref, src: object.data_url };
    })).then((value) => { if (alive) setImages(value); }).catch((e) => { if (alive) setError(e.message); });
    return () => { alive = false; };
  }, [imageReferences]);
  if (!problem) return null;
  return <><Typography.Paragraph style={{ whiteSpace: 'pre-wrap' }}>{problem.text}</Typography.Paragraph>
    {error && <Alert type="error" title={error} />}<Space wrap>{images.map((ref) => <Image key={ref.id} width={160} src={ref.src} alt={ref.name} />)}</Space></>;
}
