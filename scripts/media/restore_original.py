#!/usr/bin/env python3
"""Edit copy only; preserve original Sharingan footage, duration and AAC soundtrack.
Source: Desktop/sharingan-5k/sharingan-launch-anime-open.mp4
SHA256: 1bc9ec306b53ce9b50ab32d20f1e9304883c91867aeabc7f3e67d530c271e3a5
Requires ffmpeg with drawtext. No generated/replacement eye or music.
"""
import argparse,subprocess,hashlib
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('source',type=Path);p.add_argument('--font',required=True);a=p.parse_args()
assert hashlib.sha256(a.source.read_bytes()).hexdigest()=='1bc9ec306b53ce9b50ab32d20f1e9304883c91867aeabc7f3e67d530c271e3a5'
root=Path(__file__).resolve().parents[2];out=root/'site/media/intro.mp4'
f=[]
def box(x,y,w,h,start,end,color='0x0d0d0d'):
 f.append(f"drawbox=x={x}:y={y}:w={w}:h={h}:color={color}:t=fill:enable='gte(t,{start})*lt(t,{end})'")
def text(s,y,size,start,end,color='0xe8e8e8',fade=True):
 alpha=f":alpha='min(1,max(0,(t-{start})/0.15))*min(1,max(0,({end}-t)/0.12))'" if fade else ''
 f.append(f"drawtext=fontfile='{a.font}':text='{s}':fontsize={size}:fontcolor={color}:x=(w-text_w)/2:y={y}{alpha}:enable='gte(t,{start})*lt(t,{end})'")
# Retain every part of the eye and its movement; replace only the small tagline.
f.append("delogo=x=360:y=795:w=1200:h=130:enable='gte(t,2.3)*lt(t,2.7)'")
box(360,795,1200,130,2.7,3.7,'0x050505')
text('OPEN-SOURCE POKER AI',830,32,2.7,3.6)
# Original scene boundaries: launch-film times +1.18s after anime cold open.
box(0,0,1920,1080,14.566667,21.2)
text('OPEN CODE',310,82,14.75,18.18)
text('OPEN WEIGHTS',455,82,15.25,18.18)
text('LOCAL EVALUATION',600,72,15.75,18.18,'0xff2d3a')
text('STUDY. BUILD. PLAY.',430,104,18.5,21.2)
text('SHARINGAN.POKER',610,34,19.0,21.2,'0xff2d3a')
subprocess.run(['ffmpeg','-y','-hide_banner','-loglevel','error','-i',str(a.source),'-vf',','.join(f),'-map','0:v:0','-map','0:a:0','-c:v','libx264','-preset','fast','-crf','18','-threads','2','-c:a','copy','-movflags','+faststart',str(out)],check=True)
subprocess.run(['ffmpeg','-y','-hide_banner','-loglevel','error','-ss','1.7','-i',str(out),'-frames:v','1','-q:v','2',str(root/'site/media/intro-poster.jpg')],check=True)
print(out)
