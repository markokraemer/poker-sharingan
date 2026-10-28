"""Train an RLCard DQN on 6-max no-limit hold'em by self-play (one network, all 6 seats).

RLCard ships NO pretrained no-limit hold'em model (its model zoo has leduc/limit/uno/...),
so this is "the public learned NLHE agent you can actually obtain": the stock RLCard DQN,
trained here. Runs on the dev box only:  ~/extbots/rlcard-env/venv/bin/python -I rlcard_train.py
Env: 100 bb deep (chips_for_each=200, bb=2), 6 players, RLCard's 5 abstract actions.
"""
import sys, time, torch, numpy as np, rlcard
from rlcard.agents import DQNAgent
from rlcard.utils import reorganize

out = sys.argv[1]
minutes = float(sys.argv[2])
torch.set_num_threads(int(sys.argv[3]) if len(sys.argv) > 3 else 4)
env = rlcard.make('no-limit-holdem', config={'game_num_players': 6, 'chips_for_each': 200, 'seed': 7})
agent = DQNAgent(num_actions=env.num_actions, state_shape=env.state_shape[0], mlp_layers=[256, 256],
                 replay_memory_size=200000, replay_memory_init_size=2000, update_target_estimator_every=1000,
                 epsilon_decay_steps=int(minutes * 60 * 40 * 12 * 0.5), batch_size=128, learning_rate=5e-5,
                 train_every=4, device=torch.device('cpu'))
env.set_agents([agent] * 6)
t0 = time.time(); hands = 0; last = t0
SC = 1.0 / 20.0  # payoff scale: 10 bb = 1
while time.time() - t0 < minutes * 60:
    traj, pay = env.run(is_training=True)
    traj = reorganize(traj, pay)
    for p in range(6):
        for ts in traj[p]:
            ts = list(ts)
            ts[2] = ts[2] * SC
            agent.feed(ts)
    hands += 1
    if time.time() - last > 120:
        last = time.time()
        print(f'{(last-t0)/60:.1f} min hands {hands} ({hands/(last-t0):.1f}/s) eps_step {agent.total_t}', flush=True)
        agent.save_checkpoint('/'.join(out.split('/')[:-1]) or '.', filename=out.split('/')[-1])
agent.save_checkpoint('/'.join(out.split('/')[:-1]) or '.', filename=out.split('/')[-1])
torch.save(agent.q_estimator.qnet.state_dict(), out + '.qnet.pt')
print('done', hands, flush=True)
