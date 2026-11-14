#!/usr/bin/env python3
"""Optional Temporal worker and command-line campaign controller."""
import argparse
import asyncio
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import sys

from temporalio.client import Client
from temporalio.common import WorkflowIDReusePolicy
from temporalio.worker import Worker
from activities import Activities
from model import validate_config
from workflow import AdaptiveCampaign


async def main(args):
    client = await Client.connect(args.address)
    if args.command == "worker":
        activities = Activities(args.crash_trial, args.crash_marker)
        with ThreadPoolExecutor(max_workers=8) as executor:
            worker = Worker(client, task_queue=args.task_queue, workflows=[AdaptiveCampaign],
                            activities=[activities.dispatch, activities.reconcile, activities.observe,
                                        activities.collect_result, activities.cancel],
                            activity_executor=executor, max_cached_workflows=0)
            await worker.run()
    elif args.command == "start":
        config = json.loads(args.config.read_text())
        validate_config(config)
        handle = await client.start_workflow(AdaptiveCampaign.run, config, id=args.id,
                    task_queue=args.task_queue, id_reuse_policy=WorkflowIDReusePolicy.REJECT_DUPLICATE)
        print(json.dumps({"workflow_id": handle.id, "run_id": handle.result_run_id}))
    else:
        handle = client.get_workflow_handle(args.id)
        if args.command == "status":
            print(json.dumps(await handle.query(AdaptiveCampaign.snapshot), indent=2, sort_keys=True))
        elif args.command == "result":
            print(json.dumps(await handle.result(), indent=2, sort_keys=True))
        elif args.command == "history":
            print((await handle.fetch_history()).to_json())
        elif args.command == "retire-unknown":
            await handle.signal(AdaptiveCampaign.retire_unknown,
                                {"trial_id": args.trial_id, "reason": args.reason})
            print(json.dumps({"signal": "sent", "trial_id": args.trial_id}))


def parser():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--address", default="127.0.0.1:7233")
    cli.add_argument("--task-queue", default="ga4gh-adaptive-campaign-v1")
    commands = cli.add_subparsers(dest="command", required=True)
    worker = commands.add_parser("worker")
    worker.add_argument("--crash-trial", help="test-only: exit 86 after acceptance, before Temporal acknowledgement")
    worker.add_argument("--crash-marker", help="test-only marker path; never records remote ID")
    start = commands.add_parser("start")
    start.add_argument("--config", type=Path, required=True)
    start.add_argument("--id", required=True)
    for name in ("status", "result", "history", "retire-unknown"):
        command = commands.add_parser(name)
        command.add_argument("--id", required=True)
        if name == "retire-unknown":
            command.add_argument("--trial-id", required=True)
            command.add_argument("--reason", required=True,
                                 help="explicit risk acceptance; does not prove worker absence")
    return cli


if __name__ == "__main__":
    asyncio.run(main(parser().parse_args()))
