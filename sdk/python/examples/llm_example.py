"""LLMBot with ANY provider: you pass a function str -> str. No API keys live in the SDK."""
import os
from sharingan import run
from sharingan.bots import LLMBot

def complete(prompt: str) -> str:
    # e.g. with the anthropic SDK:
    #   import anthropic; c = anthropic.Anthropic()
    #   return c.messages.create(model=os.environ["MODEL"], max_tokens=300,
    #                            messages=[{"role": "user", "content": prompt}]).content[0].text
    raise NotImplementedError("plug in your provider")

run(LLMBot(complete), os.environ["ARENA_URL"], os.environ["ARENA_TOKEN"])
