import { forwardRef } from 'react';
import { Button } from 'antd';
import { useStore } from '../store.js';

// Panels mount after identity resolution. Standalone component previews have no
// identity and retain their existing editable behavior.
export function useCanEdit() {
  const { identity } = useStore();
  return !identity || ['admin', 'editor'].includes(identity.role);
}

export const EditButton = forwardRef(function EditButton({ disabled, ...props }, ref) {
  const canEdit = useCanEdit();
  return <Button ref={ref} {...props} disabled={!canEdit || disabled} />;
});
