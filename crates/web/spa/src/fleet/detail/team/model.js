import { decodeBase64 } from '../../model.js';

export function turnView(turn) {
  const meta = turn.meta || turn;
  return {
    number: turn.turn ?? meta.turn,
    question: typeof meta.question === 'string' ? meta.question : '本轮讨论',
    participants: Array.isArray(meta.participants) ? [...new Set(meta.participants)] : [],
    aligned: meta.aligned,
    steps: Number.isInteger(meta.sub_turns) ? Math.max(0, meta.sub_turns) : 0,
    plan: turn.detail_fields?.plan || `team.turn.${turn.turn ?? meta.turn}.plan`,
    omitted: meta.omitted ? meta : null,
  };
}

export function topicNotice(topic, status) {
  const endings = {
    complete: ['success', '讨论已完成', '队长已确认完成，最终结论如下。'],
    max_turns: ['warning', '达到讨论轮次上限', '以下保留阶段小结，尚未确认任务完成。'],
    max_sub_turns: ['warning', '达到澄清次数上限', '本轮仍有分歧，请结合小结和待澄清问题继续处理。'],
    cancelled: ['info', '讨论已取消', '已产生的发言和小结仍可查看。'],
    error: ['error', '讨论出现错误', '请查看执行错误和已产生的讨论记录。'],
  };
  if (topic.finish_reason === 'complete' && status === 'error') return endings.error;
  if (endings[topic.finish_reason]) return endings[topic.finish_reason];
  if (status === 'error') return endings.error;
  if (status === 'cancelled') return endings.cancelled;
  if (status === 'interrupted') return ['warning', '讨论已中断', '可在原节点恢复，继续已有讨论。'];
  if (topic.status === 'executing' || ['running', 'pending'].includes(status)) {
    return ['info', '讨论进行中', '队长组织问题，成员发言后由队长小结；有分歧时继续澄清。'];
  }
  return null;
}

export const clarificationMembers = (summary) => [...new Set(
  (summary.ambiguities || []).map((item) => item.node_id).filter(Boolean),
)];

// Only decode a complete bounded record. Large records retain the existing
// byte-window reader instead of silently joining unbounded responses.
export function decodeRecord(chunk) {
  if (chunk.encoding !== 'json-base64' || chunk.offset !== 0) throw new Error('讨论记录格式无效');
  const bytes = decodeBase64(chunk.bytes_b64 || '');
  if (!Number.isSafeInteger(chunk.total_bytes) || chunk.total_bytes < 0
    || bytes.length > 64 * 1024 || chunk.next_offset !== bytes.length
    || chunk.next_offset > chunk.total_bytes || chunk.eof !== (chunk.next_offset === chunk.total_bytes)) {
    throw new Error('讨论记录长度无效');
  }
  if (!chunk.eof) return { large: true };
  const value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('讨论记录格式无效');
  return { value };
}
