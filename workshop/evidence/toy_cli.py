"""Minimal process-startup probe, not an Agentboard implementation."""
import sys

if sys.argv[1:] == ["status"]:
    print("agentboard: ok")
else:
    sys.exit(2)
