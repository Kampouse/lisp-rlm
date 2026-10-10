"""Contract layout — single source of truth (renamed 2026-10-10).

Descriptive dirs, lisp+ts colocated per contract, grouped by leg.
NEAR account ids stay historical (baked into scenario asserts):
  pa.clmm.test.near etc. Only FOLDERS were renamed.
"""
import os

HERE = os.path.dirname(os.path.abspath(__file__))

MAP = {  # acct -> (leg, dirname)
    "pa":   ("leg2", "pool-a"),
    "pb":   ("leg2", "pool-b"),
    "pd":   ("leg2", "join-ab"),
    "splt": ("leg2", "split-ab"),
    "pc":   ("leg3", "pool-c"),
    "pj":   ("leg3", "fee-join-ac"),
    "n1":   ("leg4", "pool-n1"),
    "n2":   ("leg4", "pool-n2"),
    "splt3":("leg4", "split-3"),
}
ACCTS = list(MAP)

def cdir(acct):    return os.path.join(HERE, "contracts", *MAP[acct])
def lisp_wasm(a):  return os.path.join(cdir(a), "target", MAP[a][1] + ".wasm")
def ts_dir(a):     return os.path.join(cdir(a), "ts")
def ts_wasm(a):    return os.path.join(ts_dir(a), "target", MAP[a][1] + "-ts.wasm")

PROOF = os.path.join(HERE, "proof")
GENERATORS = os.path.join(HERE, "generators")
SCEN_LISP = os.path.join(HERE, "scenarios", "lisp")
SCEN_TS = os.path.join(HERE, "scenarios", "ts")

def scen_lisp(n): return os.path.join(SCEN_LISP, n)
def scen_ts(n):   return os.path.join(SCEN_TS, n)
