import subprocess, time, base64, json, urllib.request, struct, sys

RPC = 'https://rpc.testnet.fastnear.com'
CONTRACT = 'lending.kampy.testnet'
SIGNER = 'kampy.testnet'

def rpc(method, params):
    data = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":params}).encode()
    req = urllib.request.Request(RPC, data=data, headers={"Content-Type":"application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=10).read())

def deploy():
    wasm = open('burrow-mini.wasm', 'rb').read()
    b64 = base64.b64encode(wasm).decode()
    pk = rpc('query', {"request_type":"view_access_key", "finality":"final", "account_id":SIGNER, "public_key":"ed25519:3iJR1cCHyedZQbNmMhkzCyoeE7ehNbY14iGxG3o3pPqB"})
    nonce = pk['result']['nonce'] + 1
    block = rpc('block', {"finality":"final"})
    bh = block['result']['header']['hash']
    tx = {"signer_id":SIGNER,"receiver_id":CONTRACT,"actions":[{"type":"DeployContract","code":b64}],
          "nonce":nonce,"block_hash":bh}
    signed = subprocess.run(['near', 'sign-transaction', 'sign-serialized', json.dumps(tx)],
        capture_output=True, text=True, timeout=15)
    if signed.returncode != 0:
        return f"sign err: {signed.stderr[:100]}"
    tx_json = json.loads(signed.stdout)
    resp = rpc('broadcast_tx_commit', [tx_json])
    if 'error' in resp:
        return f"ERR: {resp['error']['message'][:80]}"
    return "OK" if resp.get('result',{}).get('status',{}).get('SuccessValue') else f"FAIL: {json.dumps(resp['result'])[:80]}"

def call_fn(method, args_bytes=b'', deposit=0):
    pk = rpc('query', {"request_type":"view_access_key", "finality":"final", "account_id":SIGNER, "public_key":"ed25519:3iJR1cCHyedZQbNmMhkzCyoeE7ehNbY14iGxG3o3pPqB"})
    nonce = pk['result']['nonce'] + 1
    block = rpc('block', {"finality":"final"})
    bh = block['result']['header']['hash']
    b64args = base64.b64encode(args_bytes).decode() if args_bytes else ''
    tx = {"signer_id":SIGNER,"receiver_id":CONTRACT,
          "actions":[{"type":"FunctionCall","method_name":method,"args":b64args,"gas":"300000000000000","deposit":str(deposit)}],
          "nonce":nonce,"block_hash":bh}
    signed = subprocess.run(['near', 'sign-transaction', 'sign-serialized', json.dumps(tx)],
        capture_output=True, text=True, timeout=15)
    if signed.returncode != 0:
        return f"sign err: {signed.stderr[:100]}"
    tx_json = json.loads(signed.stdout)
    resp = rpc('broadcast_tx_commit', [tx_json])
    if 'error' in resp:
        return f"ERR: {resp['error']['message'][:100]}"
    status = resp.get('result',{}).get('status',{})
    sv = status.get('SuccessValue','')
    if sv:
        raw = base64.b64decode(sv)
        return raw.decode('utf-8', errors='replace') if raw else '(empty)'
    if 'Failure' in str(status):
        return f"FAIL: {json.dumps(status.get('Failure',{}))[:100]}"
    return '(no value)'

passed = failed = 0
def check(name, fn, expected=None):
    global passed, failed
    result = fn()
    ok = expected is None or expected in str(result)
    if ok: passed += 1
    else: failed += 1
    print(f"  {'PASS' if ok else 'FAIL'}: {name} -> {str(result)[:70]}")

print("Deploying...")
print(f"  deploy -> {deploy()}")
time.sleep(1)

print("\nInit...")
check("init", lambda: call_fn('init'))
time.sleep(0.5)

print("\nRegister asset (vol=80, cf=80, tu=80, r0=100, r1=500, r2=3000, rc=25)...")
args = struct.pack('<IIIIIII', 80, 80, 80, 100, 500, 3000, 25)
check("reg_asset", lambda: call_fn('register_asset', args))
time.sleep(0.5)

print("\nDeposit 5M...")
check("deposit 5M", lambda: call_fn('deposit', struct.pack('<I', 5000000)))
time.sleep(0.5)

print("\nPool state...")
check("pool/s", lambda: call_fn('get_pool_s'), '5000000')
check("pool/a", lambda: call_fn('get_pool_a'), '5000000')
check("borrow_idx", lambda: call_fn('get_borrow_index'), '1000000')
check("supply_idx", lambda: call_fn('get_supply_index'), '1000000')

print("\nCollateral 10M...")
check("inc_collat 10M", lambda: call_fn('inc_collat', struct.pack('<I', 10000000)))
time.sleep(0.5)

print("\nBorrow 3M...")
check("borrow 3M", lambda: call_fn('borrow', struct.pack('<I', 3000000)))
time.sleep(0.5)

print("\nHealth & rates...")
check("health", lambda: call_fn('get_health'))
check("borrow_rate", lambda: call_fn('get_borrow_rate'))
check("supply_rate", lambda: call_fn('get_supply_rate'))
check("reserve", lambda: call_fn('get_reserve'), '0')

print("\nRepay 1M...")
check("repay 1M", lambda: call_fn('repay', struct.pack('<I', 1000000)))
time.sleep(0.5)

print("\nDeposit 5M more...")
check("deposit 5M", lambda: call_fn('deposit', struct.pack('<I', 5000000)))
time.sleep(0.5)

print("\nWithdraw 2M...")
check("withdraw 2M", lambda: call_fn('withdraw', struct.pack('<I', 2000000)))
time.sleep(0.5)

print("\nUnsafe borrow 50M...")
check("reject unsafe", lambda: call_fn('borrow', struct.pack('<I', 50000000)), 'unsafe')
time.sleep(0.5)

print("\nDec collateral 3M...")
check("dec_collat 3M", lambda: call_fn('dec_collat', struct.pack('<I', 3000000)))
time.sleep(0.5)

print("\nRepay remaining 2M...")
check("repay 2M", lambda: call_fn('repay', struct.pack('<I', 2000000)))
time.sleep(0.5)

print("\nFinal health...")
check("final health", lambda: call_fn('get_health'), '10000')

print(f"\n{'='*40}")
print(f"Results: {passed} passed, {failed} failed")
