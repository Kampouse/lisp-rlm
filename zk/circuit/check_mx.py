import json

# Reconstruct what mx SHOULD be and diff against expectations
init = json.load(open("/Users/j-p/dev/stuff/lisp-rlm/zk/circuit/init_args.json"))
verify = json.load(open("/Users/j-p/dev/stuff/lisp-rlm/zk/circuit/verify_args.json"))
ONE_HEX = "01" + "00" * 31

expected = init["ic"][0] + ONE_HEX
for i in range(1, len(init["ic"])):
    expected += init["ic"][i] + verify["inputs"][i - 1]

print("expected len:", len(expected))
print("chars 180..220:", expected[180:220])
print("last 40:", expected[-40:])
