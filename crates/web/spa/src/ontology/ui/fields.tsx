import { Checkbox, Form, Input, InputNumber, Select, Switch } from "antd";
import type { FormItemProps, SelectProps } from "antd";
import type { ReactNode } from "react";

type Props = Pick<FormItemProps, "name" | "label" | "rules" | "tooltip" | "initialValue"> & {
  disabled?: boolean; placeholder?: string; fieldProps?: Record<string, unknown>;
  options?: SelectProps["options"]; mode?: SelectProps["mode"]; showSearch?: boolean; children?: ReactNode;
};
function Field({ kind, ...props }: Props & { kind: "text" | "textarea" | "number" | "select" | "switch" | "check" }) {
  const { name, label, rules, tooltip, initialValue, fieldProps, children, mode, options: selectOptions, showSearch, ...control } = props;
  const options = { ...control, ...fieldProps };
  const element = kind === "textarea" ? <Input.TextArea {...options} />
    : kind === "number" ? <InputNumber style={{ width: "100%" }} {...options} />
    : kind === "select" ? <Select mode={mode} options={selectOptions} showSearch={showSearch} {...options} />
    : kind === "switch" ? <Switch {...options} />
    : kind === "check" ? <Checkbox {...options}>{children}</Checkbox> : <Input {...options} />;
  return <Form.Item name={name} label={label} rules={rules} tooltip={tooltip} initialValue={initialValue}
    valuePropName={kind === "switch" || kind === "check" ? "checked" : "value"}>{element}</Form.Item>;
}
export const TextField = (props: Props) => <Field kind="text" {...props} />;
export const TextAreaField = (props: Props) => <Field kind="textarea" {...props} />;
export const NumberField = (props: Props) => <Field kind="number" {...props} />;
export const SelectField = (props: Props) => <Field kind="select" {...props} />;
export const SwitchField = (props: Props) => <Field kind="switch" {...props} />;
export const CheckField = (props: Props) => <Field kind="check" {...props} />;
