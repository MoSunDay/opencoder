"""Cursor consumer with the UI's retry behavior across releases and crashes."""
import http.client
import threading
import time
import urllib.error
import urllib.request


class Stream:
    def __init__(self, env, identifier):
        self.ids = []
        self.reconnects = []
        self.resume_delays = []
        self.errors = []
        self.finished = threading.Event()
        self.thread = threading.Thread(target=self.consume,args=(env,identifier),daemon=True)
        self.thread.start()

    def consume(self, env, identifier):
        cursor, failures = 0, 0
        disconnected = None
        try:
            while failures < 5:
                request = urllib.request.Request(env.settings.public_url + '/api/executions/' + identifier + '/events?after=' + str(cursor),
                    headers={'Authorization':'Bearer ' + env.token,'Last-Event-ID':str(cursor)})
                try:
                    with env.opener.open(request,timeout=90) as response:
                        if disconnected is not None:
                            self.resume_delays.append(time.monotonic() - disconnected)
                            disconnected = None
                        event, seq = None, None
                        for raw in response:
                            line = raw.decode().strip()
                            if line.startswith('id:'):
                                seq = int(line[3:])
                            elif line.startswith('event:'):
                                event = line[6:].strip()
                            elif not line:
                                if event:
                                    failures = 0
                                if event == 'reconnect':
                                    self.reconnects.append(time.monotonic())
                                    disconnected = time.monotonic()
                                    break
                                if seq is not None:
                                    assert seq > cursor, (seq,cursor)
                                    cursor = seq
                                    self.ids.append(seq)
                                if event == 'stream_end':
                                    self.finished.set()
                                    return
                                event, seq = None, None
                except urllib.error.HTTPError as error:
                    if error.code < 500:
                        raise
                except (OSError, http.client.HTTPException, urllib.error.URLError):
                    pass
                failures += 1
                time.sleep(.1 if disconnected is not None else min(2 ** (failures - 1),15))
            raise RuntimeError('event stream did not recover after five attempts')
        except Exception as error:
            self.errors.append(str(error))
