import { Button, Checkbox, Select, Tooltip, Typography, theme } from "antd";
import { useEffect, useRef, useState } from "react";

type Props = {
  label: string;
  placeholder: string;
  value: string[];
  options: { value: string; label: string; description?: string }[];
  disabled?: boolean;
  onChange: (value: string[]) => void;
};

/** Collect a draft while open and commit once when the menu closes. */
export default function BatchMultiSelect({ label, placeholder, value, options, disabled, onChange }: Props) {
  const { token } = theme.useToken();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState(value);
  const [selectedOnly, setSelectedOnly] = useState(false);
  const draftRef = useRef(value);
  const previous = useRef(value);
  useEffect(() => {
    if (value !== previous.current || disabled) {
      previous.current = value;
      draftRef.current = value;
      setDraft(value);
      setOpen(false);
    }
  }, [value, disabled]);
  const close = () => {
    setOpen(false);
    const next = draftRef.current;
    if (next.length !== value.length || next.some((id) => !value.includes(id))) onChange(next);
  };

  return <Select mode="multiple" aria-label={label} allowClear showSearch
    filterOption={(input, option) => `${option?.label ?? ""} ${option?.description ?? ""} ${option?.value ?? ""}`.toLocaleLowerCase().includes(input.toLocaleLowerCase())}
    maxTagCount={1} maxTagTextLength={14}
    maxTagPlaceholder={(omitted) => <Tooltip title={omitted.map((item) => item.label).join("、")}>+{omitted.length}</Tooltip>}
    placeholder={placeholder} value={open ? draft : value} options={selectedOnly ? options.filter((item) => draft.includes(item.value)) : options} disabled={disabled} style={{ width: "100%" }}
    optionRender={(option) => <div style={{ whiteSpace: "normal", overflowWrap: "anywhere" }}><div>{option.label}</div>{option.data.description
      ? <Typography.Text type="secondary" style={{ fontSize: token.fontSizeSM }}>{option.data.description}</Typography.Text> : null}</div>}
    open={open && !disabled} onOpenChange={(next) => {
      if (next) { draftRef.current = value; setDraft(value); setSelectedOnly(false); setOpen(true); } else close();
    }} onChange={(next) => {
      draftRef.current = next; setDraft(next);
      if (!open) onChange(next);
    }}
    popupRender={(menu) => <>
      <div style={{ padding: "8px 12px" }} onMouseDown={(event) => event.preventDefault()}><Checkbox checked={selectedOnly} onChange={(e) => setSelectedOnly(e.target.checked)}>仅看已选</Checkbox></div>
      {menu}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "8px 12px", borderTop: `1px solid ${token.colorBorderSecondary}` }}>
        <Typography.Text type="secondary">已选 {draft.length} 项，收起后生效</Typography.Text>
        <Button size="small" type="primary" onMouseDown={(event) => event.preventDefault()} onClick={close}>完成</Button>
      </div>
    </>} />;
}
