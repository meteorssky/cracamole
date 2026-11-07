import unittest
from compare_args import instruction, accepted_args


class ProtocolReadTests(unittest.TestCase):
    def test_only_explicitly_allowed_appended_extension_is_accepted(self):
        oracle = b"4.args,3.foo;"
        candidate = b"4.args,3.foo,18.character-encoding;"
        self.assertFalse(accepted_args(oracle, candidate))
        self.assertTrue(accepted_args(oracle, candidate, True))
        self.assertFalse(accepted_args(oracle, b"4.args,18.character-encoding,3.foo;", True))
        self.assertFalse(accepted_args(oracle, b"4.args,3.bar,18.character-encoding;", True))
    def test_unicode_and_delimiters(self):
        self.assertEqual(instruction("4.args,4.中,;😀;trailing"), (["args", "中,;😀"], 14))

    def test_every_truncation_is_partial(self):
        wire = "4.args,3.foo,0.;"
        for i in range(len(wire)):
            self.assertIsNone(instruction(wire[:i]))
        self.assertEqual(instruction(wire), (["args", "foo", ""], len(wire)))

    def test_invalid_length_and_terminator(self):
        for wire in ["x.args;", "4.args!", "8193.x;", "１.x;"]:
            with self.assertRaises(ValueError):
                instruction(wire)


if __name__ == "__main__":
    unittest.main()
