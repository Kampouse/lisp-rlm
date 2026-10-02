#!/usr/bin/env python3
"""Answer the 5 insight questions from traces + CCG + Q-table."""
import json, glob, collections, re, datetime

def norm(v):
    if isinstance(v, str): return v == "True" or v == "true"
    return bool(v)

# ---------- load traces ----------
eps = []
for f in sorted(glob.glob('data/rlm/traces/*/*.json')):
    try: d = json.load(open(f))
    except Exception: continue
    seq = []
    for n in (d.get('nodes') or []):
        if isinstance(n, dict):
            seq.append((int(n.get('i', 0)), str(n.get('s', '')),
                        str(n.get('a', n.get('act', ''))).replace('~~QT~~', '').strip('()'),
                        norm(n.get('ok', False))))
    eps.append({'task': d.get('task_id'), 'ts': float(d.get('ts') or 0),
                'completed': norm(d.get('completed')), 'seq': sorted(seq)})
print(f"episodes parsed: {len(eps)}")

# ---------- Q2: visits vs Q-table ----------
visits = collections.Counter()
for e in eps:
    for i, s, a, ok in e['seq']:
        visits[s] += 1
Q = json.load(open('scripts/rlm-tasks/q-values.json'))
unvis = [k for k in Q if visits.get(k, 0) == 0]
lowvis = [k for k in Q if 0 < visits.get(k, 0) <= 2]
best = {k: max(v.values()) for k, v in Q.items()}
vis_neg = [k for k in Q if best[k] <= 0 and visits.get(k, 0) > 0]
print("\n== Q2: starvation vs pessimism ==")
print(f"Q states: {len(Q)} | never visited: {len(unvis)} ({len(unvis)/len(Q):.0%}) | 1-2 visits: {len(lowvis)} ({len(lowvis)/len(Q):.0%})")
print(f"positive best-Q: {sum(1 for k in Q if best[k]>0)} | visited-but-all-neg: {len(vis_neg)}")
vhist = collections.Counter(min(visits.get(k, 0), 6) for k in Q)
print("visit histogram (6=6+):", dict(sorted(vhist.items())))
print(f"states seen in traces but pruned from Q: {sum(1 for s in visits if s not in Q)}")
med = sorted(v for v in visits.values() if v and v < 99999)
if med: print(f"median visits of visited states: {med[len(med)//2]}")

# ---------- Q1: action efficacy ----------
trans = collections.defaultdict(lambda: [0, 0])
by_task = collections.defaultdict(lambda: collections.defaultdict(lambda: [0, 0]))
for e in eps:
    seq = e['seq']
    for a, b in zip(seq, seq[1:]):
        if not a[3] and b[0] == a[0] + 1:
            act = a[2] or 'none'
            trans[act][1] += 1
            by_task[e['task']][act][1] += 1
            if b[3]:
                trans[act][0] += 1
                by_task[e['task']][act][0] += 1
print("\n== Q1: P(next attempt solves | failed now, action) ==")
for act, (s, t) in sorted(trans.items(), key=lambda x: -x[1][1]):
    if t >= 15: print(f"  {act:18s} {s}/{t} = {s/t:.0%}")
wins = ties = losses = 0
for task, acts in by_task.items():
    if 'ladder' in acts and 'none' in acts and acts['ladder'][1] >= 5 and acts['none'][1] >= 5:
        lr = acts['ladder'][0]/acts['ladder'][1]; nr = acts['none'][0]/acts['none'][1]
        if lr > nr: wins += 1
        elif lr == nr: ties += 1
        else: losses += 1
print(f"  within-task ladder-vs-none: ladder wins {wins}, ties {ties}, none wins {losses}")

# ---------- Q3: discovery vs regurgitation ----------
first_solve = {}
for e in sorted(eps, key=lambda e: e['ts']):
    if e['completed'] and e['task'] not in first_solve:
        first_solve[e['task']] = e['ts']
stats = collections.defaultdict(lambda: [0, 0, 0])  # task -> [pre, post, episodes]
for e in eps:
    t = e['task']; fs = first_solve.get(t)
    stats[t][2] += 1
    if not e['completed']: continue
    if fs and e['ts'] > fs: stats[t][1] += 1
    else: stats[t][0] += 1
tot_pre = sum(s[0] for s in stats.values()); tot_post = sum(s[1] for s in stats.values())
ever = [t for t in stats if stats[t][0]+stats[t][1] > 0]
print("\n== Q3: discovery vs regurgitation ==")
print(f"total solves: {tot_pre+tot_post} | first-time: {tot_pre} | repeat: {tot_post} ({tot_post/(tot_pre+tot_post):.0%} carried)")
one_shot = [t for t in ever if stats[t][0] == stats[t][0]+stats[t][1] and stats[t][0] == 1]
grind = [(t, stats[t]) for t in ever if stats[t][2] >= 15]
grind.sort(key=lambda x: -x[1][2])
print("grind tasks (15+ eps): task: pre/post/total-solves/eps")
for t, s in grind[:10]: print(f"  {t:22s} {s[0]}/{s[1]}/{s[0]+s[1]}/{s[2]}")

# ---------- Q4: failure Pareto (CCG fail nodes) ----------
g = json.load(open('data/rlm/ccg/graph.json'))
def bucket(out):
    o = str(out)
    if 'compilation failed' in o: return 'compile/parse fail'
    if 'run_program' in o and 'compilation' not in o: return 'runtime exec'
    if 'INVISIBLE to the world' in o or 'set! answer' in o: return 'set! lint bounce'
    if 'dangling-define' in o or 'Defines alone' in o: return 'dangling define'
    if 'free-bounce' in o: return 'free-bounce'
    if 'QTQTstop' in o or 'stop' in o.lower()[:20]: return 'stopped/budget'
    if 'type' in o.lower()[:25]: return 'type error'
    if o.strip() in ('', 'nan', 'None'): return 'no-output/wrong-ans'
    return 'wrong answer (silent)'
cnt = collections.Counter()
for k, v in g['nodes']:
    if not norm(v.get('ok')): cnt[bucket(v.get('out', ''))] += 1
print("\n== Q4: failure Pareto (CCG failed attempts) ==")
tot = sum(cnt.values())
for b, c in cnt.most_common(): print(f"  {b:24s} {c:5d} ({c/tot:.0%})")

# ---------- Q5: A/B per-task ----------
print("\n== Q5: A/B per-task slice ==")
print("traces carry no arm tag (fields: policy/variant only) -> cannot slice retroactively")
