"""Confirm an actual local delivery or a completed no-ticket business decision."""


def delivered(job, job_id, records):
    if job.get('jobId') != job_id or job.get('analysis', {}).get('status') != 'done':
        return False
    if (job.get('viking') or {}).get('status') == 'not_required':
        return True
    return any(row.get('type') in ('local-delivery', 'local-viking')
               and row.get('jobId') == job_id for row in records)
