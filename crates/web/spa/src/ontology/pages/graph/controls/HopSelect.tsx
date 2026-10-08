import { Form, Select } from "antd";

type Props = {
  upstream: number;
  downstream: number;
  maximum: number;
  disabled?: boolean;
  labelPrefix?: string;
  onUpstream: (depth: number) => void;
  onDownstream: (depth: number) => void;
};

export default function HopSelect({ upstream, downstream, maximum, disabled, labelPrefix = "", onUpstream, onDownstream }: Props) {
  const options = Array.from({ length: maximum + 1 }, (_, value) => ({ value, label: `${value} 跳` }));
  return <div className="graph-hops">
    <Form.Item label="上游"><Select aria-label={`${labelPrefix}上游跳数`} value={upstream} options={options}
      disabled={disabled} onChange={onUpstream} style={{ width: "100%" }} /></Form.Item>
    <Form.Item label="下游"><Select aria-label={`${labelPrefix}下游跳数`} value={downstream} options={options}
      disabled={disabled} onChange={onDownstream} style={{ width: "100%" }} /></Form.Item>
  </div>;
}
