"""Separate transport/execution success from evidence-backed business quality."""


def quality_results(evaluation, regression):
    executions = regression.get('executions') or []
    prepared = [e for e in executions if not e.get('error') and not e.get('timedOut')]
    dependency_failures = sum(e.get('error') == 'Dependency preparation failed' for e in executions)
    evaluation_valid = (evaluation.get('health') in ['clean', 'recovered', 'failed', 'internal_failure']
                        and evaluation.get('gaps') == []
                        and (evaluation.get('findingCount') == 0
                             or (evaluation.get('reportReady') is True and bool(evaluation.get('conclusions')))))
    regression_valid = (regression.get('verdict') in ['pass', 'block'] and bool(prepared)
                        and len(prepared) == len(executions))
    return {
        'eval-diagnose': {'valid': evaluation_valid, 'health': evaluation.get('health'),
                          'reportReady': evaluation.get('reportReady', False)},
        'regression-test': {'valid': regression_valid, 'verdict': regression.get('verdict'),
                            'testAttemptCount': len(executions),
                            'testCommandsSucceeded': sum(e.get('exitCode') == 0 for e in prepared),
                            'dependencyPreparationFailures': dependency_failures},
    }


def acceptance_passed(platform_passed, browser_passed, quality):
    return (platform_passed is True and browser_passed is True
            and set(quality) == {'eval-diagnose', 'regression-test'}
            and all(result.get('valid') is True for result in quality.values()))
