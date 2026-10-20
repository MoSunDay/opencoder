from pathlib import Path
import json
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from rolling.maintenance.engine import images


class RetainedImageReceiptTests(unittest.TestCase):
    def image(self, path, content):
        (path / 'usr/bin').mkdir(parents=True)
        for name in images.RUNNERS:
            (path / 'usr/bin' / name).write_text(content + name)
        return path

    def test_successive_upgrades_retain_all_original_image_identities(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            original = self.image(root / 'dag/rootfs', 'initial')
            first = self.image(root / 'programs/first/rootfs', 'first')
            second = self.image(root / 'programs/second/rootfs', 'second')
            images.stage(root, original, first)
            receipt = Path(images.stage(root, first, second))
            value = images.read(receipt, root, second)
            self.assertEqual(set(value['source_rootfs']), {str(original), str(first)})
            self.assertEqual(images.stage(root, first, second), str(receipt))
            self.assertEqual((original / 'usr/bin/dag-runner').read_text(), 'initialdag-runner')

    def test_earlier_engine_upgrade_without_receipt_keeps_the_first_frozen_image(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            original = self.image(root / 'dag/rootfs', 'initial')
            previous = self.image(root / 'programs/previous/rootfs', 'previous')
            candidate = self.image(root / 'programs/candidate/rootfs', 'candidate')
            value = json.loads(Path(images.stage(root, previous, candidate)).read_text())
            self.assertEqual(set(value['source_rootfs']), {str(original), str(previous)})

    def test_modified_previous_runner_or_receipt_prevents_chained_upgrade(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            original = self.image(root / 'dag/rootfs', 'initial')
            previous = self.image(root / 'programs/previous/rootfs', 'previous')
            candidate = self.image(root / 'programs/candidate/rootfs', 'candidate')
            images.stage(root, original, previous)
            (previous / 'usr/bin/dag-runner').write_text('changed')
            with self.assertRaisesRegex(ValueError, 'does not match'):
                images.stage(root, previous, candidate)

    def test_existing_receipt_cannot_be_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            original = self.image(root / 'dag/rootfs', 'initial')
            target = self.image(root / 'programs/candidate/rootfs', 'candidate')
            path = Path(images.stage(root, original, target))
            value = json.loads(path.read_text())
            value['source_rootfs'] = ['/unrelated']
            path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, 'receipt changed'):
                images.stage(root, original, target)


if __name__ == '__main__':
    unittest.main()
