"""Private host API transport; credentials never appear in tool results."""
import json
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import urlparse
from urllib.request import ProxyHandler, Request, build_opener


class Client:
    def __init__(self, endpoint='http://127.0.0.1:18081', token_file='/etc/opencoder/server.token'):
        parsed = urlparse(endpoint)
        if parsed.scheme not in ('http', 'https') or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError('Invalid control endpoint')
        self.endpoint = endpoint.rstrip('/')
        self.token_file = Path(token_file)
        self.opener = build_opener(ProxyHandler({}))

    def call(self, method, path, body=None):
        payload = None if body is None else json.dumps(body, ensure_ascii=False).encode()
        request = Request(self.endpoint + path, data=payload, method=method, headers={
            'Authorization': 'Bearer ' + self.token_file.read_text().strip(),
            'Content-Type': 'application/json',
        })
        try:
            with self.opener.open(request, timeout=45) as reply:
                return json.load(reply)
        except HTTPError as error:
            # The server error, never request headers or credentials.
            try:
                detail = json.load(error).get('error', 'request rejected')
            except (ValueError, AttributeError):
                detail = 'request rejected'
            raise RuntimeError(f'HTTP {error.code}: {detail}') from None

    def submit(self, request):
        # Control owns the same-ID fingerprint and admission recovery.
        return self.call('POST', '/api/executions', request)
