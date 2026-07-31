"""Keep pymobiledevice3's unused interactive shell out of desktop bundles."""

from __future__ import annotations

import sys
import types


def _unsupported_interactive_shell(*_args, **_kwargs):
    raise RuntimeError("interactive pymobiledevice3 shell is not part of Riviu runtime")


ipython = types.ModuleType("IPython")
ipython.start_ipython = _unsupported_interactive_shell
sys.modules.setdefault("IPython", ipython)
