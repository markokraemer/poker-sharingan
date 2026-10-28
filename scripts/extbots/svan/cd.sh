#!/bin/bash
SVAN_LEARN=0 ~/svan/tools/run.sh abl-a-cold svan B 100000 11002
SVAN_LEARN=0 ~/svan/tools/run.sh abl-b-cold B svan 100000 12002
~/svan/tools/run.sh c-search-vs-svan search svan 40000 13101 --set flop=0
~/svan/tools/run.sh d-svan-vs-search svan search 12000 14101 --set flop=0
