"""The bot the local e2e submits: the Python SDK's built-in tight-aggressive example bot, as an uploadable single file.

Runs unchanged over every transport: `sharingan pipe` (hosted match worker), `sharingan connect` (WebSocket), `sharingan eval`.
"""
from sharingan.bots import TightAggressive

bot = TightAggressive()
