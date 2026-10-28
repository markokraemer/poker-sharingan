"""JSON-lines bridge server around an RLCard no-limit hold'em DQN (see arena/bench/src/ext.rs
for the protocol). argv[1] = checkpoint written by rlcard_train.py. Dev box only; run with -I.

RLCard NLHE observation (rlcard/envs/nolimitholdem.py): 54-vector = one-hot of board+hole cards
(52) + my chips committed + max chips committed (RLCard units; we scale our chips by 1/50 so a
100 bb stack = 200 chips, bb = 2, matching the training env). Five abstract actions:
0 fold, 1 check/call, 2 raise half pot, 3 raise pot, 4 all-in.
"""
import sys, json, os, torch, numpy as np
from collections import OrderedDict
import rlcard
from rlcard.agents import DQNAgent

torch.set_num_threads(1)
ck = torch.load(sys.argv[1], weights_only=False)
agent = DQNAgent.from_checkpoint(ck)
c2i = json.load(open(os.path.join(rlcard.__path__[0], 'games/limitholdem/card2index.json')))
SC = 1.0 / 50.0
SUIT = {'s': 'S', 'h': 'H', 'd': 'D', 'c': 'C'}


def conv(c):  # "As" -> "SA"
    return SUIT[c[1]] + c[0]


def decide(r):
    cards = [conv(c) for c in r['board'] + r['hole']]
    obs = np.zeros(54)
    obs[[c2i[c] for c in cards]] = 1
    me = r['seat']
    obs[52] = r['contrib'][me] * SC
    obs[53] = max(r['contrib']) * SC
    L = r['legal']
    legal = [0, 1]
    if L['raise']:
        legal += [2, 3, 4]
    st = {'obs': obs, 'legal_actions': OrderedDict((a, None) for a in legal), 'raw_legal_actions': list(legal)}
    a, _ = agent.eval_step(st)
    a = int(a)
    pot, call, cur = r['pot'], r['call_amount'], r['cur_bet']
    if a == 0:
        return {'action': 'fold'}
    if a == 1:
        return {'action': 'call'}
    if a == 2:
        return {'action': 'raise', 'to': int(cur + 0.5 * (pot + call))}
    if a == 3:
        return {'action': 'raise', 'to': int(cur + (pot + call))}
    return {'action': 'allin'}


for line in sys.stdin:
    r = json.loads(line)
    try:
        out = decide(r)
    except Exception as e:  # never die: report and let the bridge fall back
        print(json.dumps({'id': r['id'], 'error': str(e)}), flush=True)
        continue
    out['id'] = r['id']
    print(json.dumps(out), flush=True)
