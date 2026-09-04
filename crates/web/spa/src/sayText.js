// sayText.js — Say 头部 preview 与正文首行去重的纯逻辑（无 React/antd，
// 可被纯 JS 用例直接测）。stepsBlock.jsx 的 Say 头部标签与 transcript.jsx
// 的正文渲染共用这里，保证「头部 preview」与「被跳过的首行」永远出自同一
// 份拼接口径，不会两处各算各的。

/// Say 头部 preview：Say 的全部非 image 文本部分拼接后的首个非空行
/// （trim）。`image:true` 标记行是展示位，不算 Say 文本（TUI 对齐：Image
/// 块从不关闭子轮），也不进 preview；空/纯空白 Say 的 preview 为空串。
export function sayPreview(say) {
  const text = (Array.isArray(say) ? say : [])
    .filter((part) => part && part.kind === 'text'
      && typeof part.text === 'string' && !part.image)
    .map((part) => part.text)
    .join('');
  const line = text.split('\n').find((candidate) => candidate.trim() !== '');
  return line ? line.trim() : '';
}

/// 正文首行去重：头部标签已经渲染了 preview（即正文的首个非空行），正文
/// 再原样渲染就会一字不差地重复 —— 单行 Say 尤其明显。返回去掉该首行后的
/// 新 say 部分数组（输入不修改）：
///   * 文本部分按 sayPreview 的同一拼接口径定位首个非空行，trim 相等才
///     跳过（preview 无截断，按完整首行比较；首行与 preview 不一致时正文
///     全量保留）；首行之前的前导空行原样保留；
///   * 首行可能跨文本部分拼接（中间夹 image 标记行），由 carry 承接；
///   * 去完后为空（纯空白）的文本部分整个丢弃 —— 单行 Say 的正文块因此
///     为空数组，调用方不渲染任何正文/间距；
///   * think/sys/image 标记部分原样保留（它们不属于 preview 口径）。
export function sayBodyParts(say, preview) {
  const target = typeof preview === 'string' ? preview.trim() : '';
  const parts = Array.isArray(say) ? say : [];
  const out = [];
  let carry = ''; // 首行尚未终结（没遇到换行）时跨部分累积的文本
  let firstDone = false; // 首个非空行已处理（无论是否被跳过）
  let lastTextPart = null;
  for (const part of parts) {
    const isSayText = !!(part && part.kind === 'text'
      && typeof part.text === 'string' && !part.image);
    if (!isSayText || firstDone) {
      out.push(part);
      continue;
    }
    lastTextPart = part;
    const lines = (carry + part.text).split('\n');
    carry = lines.pop(); // 末段可能还没终结 —— 留给下一个文本部分拼接
    const kept = [];
    let seenNonBlank = false;
    for (const line of lines) {
      if (seenNonBlank) {
        kept.push(line);
      } else if (line.trim() === '') {
        kept.push(line); // 首个非空行之前的前导空行原样保留
      } else {
        seenNonBlank = true;
        // 与 sayPreview 同口径：trim 相等才跳过该首行。
        if (!(target !== '' && line.trim() === target)) {
          kept.push(line);
        }
      }
    }
    if (seenNonBlank) {
      firstDone = true;
      const text = kept.concat(carry).join('\n');
      carry = '';
      if (text.trim() !== '') {
        out.push({ ...part, text });
      }
    }
  }
  if (!firstDone && carry.trim() !== '') {
    // 整个正文没有换行：唯一一行就是首个非空行（完整地留在 carry 里）。
    if (!(target !== '' && carry.trim() === target)) {
      out.push({ ...lastTextPart, text: carry });
    }
  }
  return out;
}
