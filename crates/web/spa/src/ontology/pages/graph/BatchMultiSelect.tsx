import { Button, Select } from "antd";
import { useEffect, useState } from "react";

type Props = {
  label: string;
  placeholder: string;
  value: string[];
  options: { value: string; label: string }[];
  disabled?: boolean;
  onChange: (value: string[]) => void;
};

/** Select several options in one open menu, then close it explicitly. */
export default function BatchMultiSelect({ label, placeholder, value, options, disabled, onChange }: Props) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  return <Select mode="multiple" aria-label={label} allowClear showSearch optionFilterProp="label" maxTagCount={2}
    placeholder={placeholder} value={value} options={options} disabled={disabled} style={{ width: "100%" }}
    open={open && !disabled} onOpenChange={setOpen} onChange={onChange}
    popupRender={(menu) => <>
      {menu}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "8px 12px", borderTop: "1px solid #f0f0f0" }}>
        <span>已选 {value.length} 项</span>
        <Button size="small" type="primary" onMouseDown={(event) => event.preventDefault()} onClick={() => setOpen(false)}>完成</Button>
      </div>
    </>} />;
}
