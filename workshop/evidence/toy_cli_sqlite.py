"""Python startup probe that imports sqlite3 but does not open a database."""
import sqlite3
import sys

if sys.argv[1:] == ["status"]:
    print("agentboard: ok")
else:
    sys.exit(2)
