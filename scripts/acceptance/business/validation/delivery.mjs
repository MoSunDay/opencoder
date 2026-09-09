// Inject local delivery while keeping both supported business API contracts explicit.
export function deliveryAdapter(defaults, record, vikingUrl) {
  const analyze = async () => { throw Error('API must dispatch through opencoder'); };
  const regressionAnalyze = async () => { throw Error('Regression API must dispatch through opencoder'); };
  if (typeof defaults.createTicket === 'function') {
    if (typeof vikingUrl !== 'function') throw Error('Ticket URL contract is missing');
    const ticketId = '11111111-1111-4111-8111-111111111111';
    const receipt = async job => {
      await record({ type: 'local-viking', jobId: job.id ?? null, ticketId });
      return { ticket_id: ticketId, viking_url: vikingUrl(ticketId) };
    };
    const createTicket = async (_config, job) => {
      if (job.analysis?.status !== 'done') throw Error('Ticket preceded completed analysis');
      return receipt(job);
    };
    return { operations: { ...defaults, analyze, createTicket }, regressionAnalyze,
      regressionCreateTicket: async (_config, content) => receipt(content) };
  }
  if (typeof defaults.createGroup !== 'function') throw Error('Unknown business delivery contract');
  const delivered = async (type, job) => {
    if (job.analysis?.status !== 'done') throw Error('Delivery preceded completed analysis');
    await record({ type, jobId: job.id, analysis: job.analysis.status, attempt: job.analysis.attempts });
  };
  return { operations: { ...defaults, analyze,
    createGroup: async (_, job) => { await delivered('local-create', job); return {
      ticketId: 'e2e-local-ticket', chatId: 'e2e-local-chat', robotId: 'e2e-local-robot', memberCount: 2 }; },
    ensureMembers: async (_, job) => { await delivered('local-members', job); return job.delivery.group; },
    pushResult: async (_, job) => { await delivered('local-delivery', job); return 'e2e-local-message'; },
  }, regressionAnalyze };
}
