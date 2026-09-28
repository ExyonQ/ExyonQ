# AUDITOR must-fail: docstring does not hide an executable mock call.


def test():
    """No mock allowed."""
    return create_mock_backend()
