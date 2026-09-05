"""HTTP validation and OpenAI contract tests. No real API calls."""
import base64
import http.client
import io
import json
import queue
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import server as app


class ApiTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.paths = patch.multiple(app, DATA=root, IMAGES=root / "images", JOBS_FILE=root / "jobs.jsonl")
        self.paths.start()
        self.store = app.Store()
        self.q = queue.Queue()
        self.server = app.ThreadingHTTPServer(("127.0.0.1", 0), app.make_handler(self.store, self.q))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.paths.stop()
        self.tmp.cleanup()

    def request(self, body, headers=None):
        conn = http.client.HTTPConnection(*self.server.server_address, timeout=2)
        try:
            conn.request("POST", "/submit", json.dumps(body), headers or {})
            response = conn.getresponse()
            return response.status, json.loads(response.read())
        finally:
            conn.close()

    def test_invalid_types_return_400_without_enqueuing(self):
        for body in ([], None, 1, {"sentence": 123}, {"words": [1]}, {"device": [], "sentence": "hello"}):
            with self.subTest(body=body):
                self.assertEqual(self.request(body)[0], 400)
        self.assertTrue(self.q.empty())

    def test_invalid_length_returns_400(self):
        self.assertEqual(self.request({}, {"Content-Length": "oops"})[0], 400)

    def test_oversized_body_returns_413(self):
        self.assertEqual(self.request({"sentence": "a" * 17000})[0], 413)

    def test_valid_cli_sentence_is_not_silently_truncated(self):
        sentence = " ".join(["a" * 64] * 6)
        status, job = self.request({"device": "A", "sentence": sentence})
        self.assertEqual(status, 200)
        self.assertEqual(self.store.get(job["id"])["sentence"], sentence)

    def test_job_survives_leaving_latest_list(self):
        _, first = self.request({"device": "A", "sentence": "first"})
        for i in range(20):
            self.store.add("B", str(i))
        self.store.update(first["id"], status="done", image="/image/1.jpg")
        self.assertNotIn(first["id"], [j["id"] for j in self.store.latest()["items"]])
        conn = http.client.HTTPConnection(*self.server.server_address, timeout=2)
        try:
            conn.request("GET", f"/jobs/{first['id']}")
            response = conn.getresponse()
            self.assertEqual(response.status, 200)
            self.assertEqual(json.loads(response.read())["status"], "done")
        finally:
            conn.close()

    def test_openai_request_and_jpeg_response(self):
        jpeg = b"\xff\xd8test\xff\xd9"
        def respond(request, timeout):
            self.assertEqual(request.full_url, "https://api.openai.com/v1/images/generations")
            self.assertEqual(request.get_method(), "POST")
            body = json.loads(request.data)
            self.assertEqual(body, {"model": "gpt-image-1", "prompt": "hello", "n": 1,
                                   "size": "1024x1024", "quality": "low",
                                   "output_format": "jpeg", "output_compression": 60})
            return io.BytesIO(json.dumps({"data": [{"b64_json": base64.b64encode(jpeg).decode()}]}).encode())
        with patch.dict(app.os.environ, {"OPENAI_API_KEY": "test-only"}), patch.object(app.urllib.request, "urlopen", side_effect=respond):
            self.assertEqual(app.gen_openai("hello", "low", "gpt-image-1"), jpeg)

    def test_worker_reports_failure_and_processes_next_job(self):
        class Generator:
            def generate(self, sentence):
                if sentence == "fail":
                    raise RuntimeError("upstream unavailable")
                return b"\xff\xd8test\xff\xd9"
        app.Worker(self.store, self.q, Generator()).start()
        _, failed = self.request({"sentence": "fail"})
        _, succeeded = self.request({"sentence": "succeed"})
        deadline = time.monotonic() + 3
        while self.store.get(succeeded["id"])["status"] not in ("done", "error") and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(self.store.get(failed["id"])["status"], "error")
        self.assertEqual(self.store.get(succeeded["id"])["status"], "done")
        self.assertTrue((app.IMAGES / f"{succeeded['id']}.jpg").read_bytes().startswith(b"\xff\xd8"))


if __name__ == "__main__":
    unittest.main()
