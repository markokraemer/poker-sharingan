"""Poker Sharingan bot SDK. Quickstart: subclass `Bot`, implement `act(state) -> Action`, call `run(...)`."""
from .bot import Bot
from .client import ApiException, ArenaClient, Runner, run, ws_url
from .helpers import (all_in_action, bb_multiple_to, call_or_check, clamp_raise_to, legal_types, legalize,
                      parse_action_text, pot_odds, pot_raise_to, preflop_strength, render_state, safe_action)
from .models import (Action, ActionEvent, ApiError, CallOption, HandEnd, HandStart, Legal, PlayerInfo,
                     PlayerState, Pot, RaiseOption, Seat, StreetEvent, Table, YourTurn)
from .webhook import handle_webhook, serve_webhook

__version__ = "0.1.0"
__all__ = [n for n in dir() if not n.startswith("_")]
