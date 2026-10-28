import json,glob,sys
d=sys.argv[1]
fs=[json.load(open(f)) for f in glob.glob(d+"/w*.json")]
tot={}
for k in ["decisions","replay_mismatch","replay_err","panics","fold_when_free","raise_unavailable","observe_err"]:
    tot[k]=sum(x[k] for x in fs)
print("workers",len(fs),tot)
if fs:
    n=max(1,tot["decisions"])
    print("ms_mean",round(sum(x["ms_mean"]*x["decisions"] for x in fs)/n,2),"median-of-p50",sorted(x["ms_p50"] for x in fs)[len(fs)//2],"max-p95",max(x["ms_p95"] for x in fs),"max-p99",max(x["ms_p99"] for x in fs),"max",max(x["ms_max"] for x in fs))
    acts={}
    for x in fs:
        for k,v in x["acts"].items(): acts[k]=acts.get(k,0)+v
    print("acts",dict(sorted(acts.items())))
    pre=sum(x["me_preflop"]["hands"] for x in fs)
    print("SvanBot first preflop decision: n",pre,"VPIP",round(sum(x["me_preflop"]["vpip"]*x["me_preflop"]["hands"] for x in fs)/max(1,pre),3),"PFR",round(sum(x["me_preflop"]["pfr"]*x["me_preflop"]["hands"] for x in fs)/max(1,pre),3))
    b=max(fs,key=lambda x:x["observed_hands"])
    print("observed hands (largest worker)",b["observed_hands"],"entrant stats (seat-hands, vpip, pfr):",{k:(v["hands"],round(v["vpip"],3),round(v["pfr"],3)) for k,v in b["entrants"].items()})
