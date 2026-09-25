"""[GPT-6] Synthetic diagnostic protocol controls; no Rust subprocess is invoked."""
import copy
import unittest
import remote_checks


class DiagnosticProtocolTests(unittest.TestCase):
    def test_missing_wrong_span_and_infrastructure_are_not_compile_fail_evidence(self):
        message = {"message":"ValueType can only be derived for structs", "level":"error",
                   "spans":[{"is_primary":True, "file_name":"src/lib.rs"}]}
        remote_checks.assert_diagnostic([message], "ValueType can only be derived for structs")
        for bad in ([], [{"message":"linker unavailable", "spans":[]}],
                    [dict(message, spans=[{"is_primary":False, "file_name":"src/lib.rs"}])],
                    [dict(message, spans=[{"is_primary":True, "file_name":"dependency.rs"}])]):
            with self.assertRaises(ValueError):
                remote_checks.assert_diagnostic(bad, "ValueType can only be derived for structs")
        panic = copy.deepcopy(message)
        panic["message"] = "proc-macro derive panicked: ValueType can only be derived for structs"
        with self.assertRaisesRegex(ValueError, "macro panic"):
            remote_checks.assert_diagnostic([panic], "ValueType can only be derived for structs")


if __name__ == "__main__":
    unittest.main()
