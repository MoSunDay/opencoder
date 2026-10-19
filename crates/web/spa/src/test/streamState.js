import { emptyStream } from '../reduce.js';

export const optimisticUserState = (text) => ({
  ...emptyStream(),
  pendingEcho: text,
  turns: [{ kind: 'text', role: 'user', text, optimistic: true }],
});
