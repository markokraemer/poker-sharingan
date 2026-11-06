#!/usr/bin/env python3
"""Render the silent 18s project introduction. Requires Pillow, ffmpeg and a TTF.
Usage: python3 scripts/media/render_intro.py --font /path/to/mono.ttf
No external assets, network calls, model inference or third-party footage.
"""
import argparse, math, subprocess
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
p=argparse.ArgumentParser();p.add_argument('--font',required=True);a=p.parse_args()
ROOT=Path(__file__).resolve().parents[2];OUT=ROOT/'site/media';OUT.mkdir(exist_ok=True)
W,H,FPS,DURATION=960,540,24,18
fonts={n:ImageFont.truetype(a.font,n) for n in (12,14,16,20,24,32,42,50)}
red='#d0141e'; paper='#f5f3ef'; ink='#151515'; muted='#8c8983'
def ease(x):
 x=max(0,min(1,x));return x*x*(3-2*x)
def text(d,xy,s,n=16,c=ink,anchor=None):d.text(xy,s,font=fonts[n],fill=c,anchor=anchor)
def eye(d,x,y,r,t):
 d.ellipse((x-r,y-r,x+r,y+r),fill=red)
 d.ellipse((x-r*.8,y-r*.8,x+r*.8,y+r*.8),outline=ink,width=2)
 for i in range(3):
  angle=t*.65+i*math.tau/3;cx=x+math.cos(angle)*r*.52;cy=y+math.sin(angle)*r*.52
  d.ellipse((cx-r*.15,cy-r*.15,cx+r*.15,cy+r*.15),fill=ink)
 d.ellipse((x-r*.19,y-r*.19,x+r*.19,y+r*.19),fill=ink)
def frame(t):
 dark=4.5<=t<9;bg=ink if dark else paper;fg=paper if dark else ink
 im=Image.new('RGB',(W,H),bg);d=ImageDraw.Draw(im)
 for y in (48,490):d.line((36,y,924,y),fill='#353535' if dark else '#d8d5cf',width=1)
 text(d,(36,20),'POKER SHARINGAN',14,fg);text(d,(924,20),'OPEN SOURCE / APACHE-2.0',12,fg,'ra')
 d.line((36,491,36+888*t/DURATION,491),fill=red,width=2)
 text(d,(36,510),'SIX-PLAYER NO-LIMIT HOLD’EM',12,fg);text(d,(924,510),f'{min(4,int(t/4.5)+1):02d} / 04',12,fg,'ra')
 if t<4.5:
  eye(d,480,215,82+8*ease(t/2),t)
  text(d,(480,337),'POKER SHARINGAN',42,fg,'mm')
  text(d,(480,389),'An open reproduction of the Pluribus method.',16,muted,'mm')
 elif t<9:
  u=t-4.5;text(d,(480,86),'SIX SEATS. HIDDEN INFORMATION.',24,fg,'mm')
  d.rounded_rectangle((250,160,710,355),radius=90,outline='#54504b',width=2)
  labels=['As','Kd','7h','2c','9s']
  for i,label in enumerate(labels):
   x=340+i*58;y=227-int(8*math.sin(u*1.5+i*.5))
   if u>i*.12:
    d.rounded_rectangle((x,y,x+48,y+65),radius=4,fill=paper)
    text(d,(x+24,y+32),label,20,red if label[-1] in 'hd' else ink,'mm')
  for i in range(6):
   ang=math.tau*i/6;x=480+275*math.cos(ang);y=258+123*math.sin(ang)
   d.ellipse((x-20,y-20,x+20,y+20),fill=red if i==int(u)%6 else '#353535')
   text(d,(x,y),str(i+1),16,paper,'mm')
  text(d,(480,424),'Blueprint strategy + optional real-time search',16,fg,'mm')
 elif t<13.5:
  u=t-9;text(d,(65,105),'OPEN THE BLACK BOX.',32,fg)
  for i,(heading,sub) in enumerate([('CODE','Rust training and inference'),('WEIGHTS','SHARINGAN-1 / SHARINGAN-2'),('EVALUATION','Duplicate deals + AIVAT')]):
   y=194+i*79
   d.line((65,y+55,895,y+55),fill='#d8d5cf')
   if u>i*.25:
    text(d,(65,y),f'0{i+1}',16,red);text(d,(120,y-4),heading,24);text(d,(895,y+3),sub,14,muted,'ra')
 else:
  eye(d,480,160,45,t)
  text(d,(480,261),'STUDY. BUILD. EVALUATE.',32,fg,'mm')
  text(d,(480,316),'Code, trained weights and reproducible tools.',16,muted,'mm')
  text(d,(480,382),'sharingan.poker',24,red,'mm')
  text(d,(480,438),'Research models · known limitations documented',12,muted,'mm')
 return im
cmd=['ffmpeg','-y','-hide_banner','-loglevel','error','-f','rawvideo','-pix_fmt','rgb24','-s',f'{W}x{H}','-r',str(FPS),'-i','-','-an','-c:v','libx264','-threads','2','-preset','veryfast','-crf','21','-pix_fmt','yuv420p','-movflags','+faststart',str(OUT/'intro.mp4')]
proc=subprocess.Popen(cmd,stdin=subprocess.PIPE)
for i in range(FPS*DURATION):proc.stdin.write(frame(i/FPS).tobytes())
proc.stdin.close();assert proc.wait()==0
frame(2).save(OUT/'intro-poster.jpg',quality=92)
for i,t in enumerate([2,6.5,11,16]):frame(t).save(OUT/f'preview-{i}.png')
print('Rendered',OUT/'intro.mp4')
