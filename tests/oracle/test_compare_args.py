import unittest
from compare_args import instruction


class ProtocolReadTests(unittest.TestCase):
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
