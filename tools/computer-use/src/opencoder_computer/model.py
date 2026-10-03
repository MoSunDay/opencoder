"""Request metadata for Cua's generic loop; Cua still predicts and executes actions."""

import base64
import copy
import io
import json
import math


def validate_native_response(response: dict) -> None:
    from cua_agent.loops.generic_vlm import (
        QWEN3_COMPUTER_TOOL,
        convert_qwen_tool_args_to_computer_action,
    )

    schema = QWEN3_COMPUTER_TOOL["function"]["parameters"]["properties"]
    for choice in response.get("choices", []):
        message = choice.get("message", {})
        content = message.get("content") or ""
        if any(tag in content for tag in ("<tool_call>", "<invoke", "<function_calls>")):
            raise RuntimeError("Native model returned embedded tool text instead of API tool_calls")
        calls = message.get("tool_calls") or []
        if len(calls) > 1:
            raise RuntimeError("Native model returned parallel computer calls")
        for call in calls:
            function = call.get("function", {})
            args = json.loads(function.get("arguments", "{}"))
            if (function.get("name") != "computer" or not isinstance(args, dict)
                    or set(args) - set(schema)
                    or args.get("action") not in schema["action"]["enum"]):
                raise RuntimeError("Native model arguments do not match Cua's computer schema")
            coordinate = args.get("coordinate")
            if coordinate is not None and (
                not isinstance(coordinate, list) or len(coordinate) != 2
                or any(isinstance(n, bool) or not isinstance(n, (int, float))
                       or not math.isfinite(n) or not 0 <= n <= 1000 for n in coordinate)
            ):
                raise RuntimeError("Native model coordinates must be numeric values in 0..1000")
            if args["action"] not in {"screenshot", "wait"} and (
                convert_qwen_tool_args_to_computer_action(args) is None
            ):
                raise RuntimeError("Native model arguments are missing required action fields")


def generation_options(native: bool, os_type: str | None = None) -> dict:
    if not native:
        return {}
    from cua_agent.loops.generic_vlm import QWEN3_COMPUTER_TOOL

    tool = copy.deepcopy(QWEN3_COMPUTER_TOOL)
    tool["function"]["parameters"]["properties"]["coordinate"]["description"] = (
        "[x, y] normalized to 0..1000 using the reference dimensions in the user instruction"
    )
    if os_type == "windows":
        tool["function"]["parameters"]["properties"]["pixels"]["description"] = (
            "Native Windows wheel ticks, NOT pixels. Use small amounts such as 3 or 5. "
            "Positive=up, negative=down. Ctrl+End can scroll to the document bottom."
        )
    return {"extra_body": {
        "tools": [tool], "parallel_tool_calls": False,
    }}


def check_dependencies(agent, native: bool) -> None:
    from cua_agent.loops.generic_vlm import GenericVlmConfig

    generic = isinstance(agent.agent_loop, GenericVlmConfig)
    if native and not generic:
        raise ValueError("native_tool_calls requires Cua's generic vision loop")
    if generic:
        try:
            import qwen_agent  # noqa: F401
            import qwen_vl_utils  # noqa: F401
        except ImportError as error:
            raise RuntimeError("Generic vision requires cua-agent[qwen]") from error


def coordinate_instruction(messages: list[dict]) -> str | None:
    from cua_agent.responses import convert_responses_items_to_completion_messages
    from PIL import Image
    from qwen_vl_utils import smart_resize

    converted = convert_responses_items_to_completion_messages(
        messages, allow_images_in_tool_results=False
    )
    for message in reversed(converted):
        content = message.get("content")
        if not isinstance(content, list):
            continue
        for item in reversed(content):
            if item.get("type") != "image_url":
                continue
            url = item["image_url"]["url"]
            if not url.startswith("data:"):
                raise ValueError("Native tool coordinates require an inline screenshot")
            with Image.open(io.BytesIO(base64.b64decode(url.split(",", 1)[1]))) as image:
                height, width = smart_resize(
                    image.height, image.width, factor=32,
                    min_pixels=3136, max_pixels=12845056,
                )
            return (
                "Return computer calls in the API tool_calls field; never write tool tags "
                "or tool JSON in the assistant content. "
                'Call the computer function with Qwen arguments: {"action":"left_click", '
                '"coordinate":[500,500]}. Use action and coordinate fields, NEVER type, x, y, '
                'or camelCase action names. Use the exact action names in the function schema. '
                'For keyboard shortcuts use action="key" and keys; for typing use action="type" '
                'and text. Return at most one tool call per turn and inspect its result before '
                'the next action. The conversation may show internal Cua action formats; '
                'do not imitate those internal formats in function arguments. '
                "Computer tool coordinates MUST use Cua's normalized 0..1000 reference space, "
                "NOT raw screenshot pixels. First locate the element's pixel position, then "
                f"multiply x by 1000/{width} and y by 1000/{height}. "
                "Use those normalized coordinates in every computer tool call. "
                "When the requested outcome is visibly verified, return a plain final answer."
            )
    return None


class NativeCoordinates:
    def __init__(self, screenshot=None, os_type=None):
        self.screenshot = screenshot
        self.os_type = os_type

    async def on_api_end(self, kwargs: dict, result: dict) -> None:
        validate_native_response(result)

    async def on_llm_start(self, messages: list[dict]) -> list[dict]:
        instruction = coordinate_instruction(messages)
        history = any(item.get("type") == "computer_call_output" for item in messages)
        if (instruction is None or history) and self.screenshot is not None:
            from cua_agent.responses import make_input_image_item

            messages = [*messages, make_input_image_item(await self.screenshot())]
            instruction = coordinate_instruction(messages)
        if instruction is None:
            return messages
        if self.os_type == "windows":
            instruction += (
                " Native Windows scroll amounts are wheel ticks, not screen pixels. "
                "Use small deltas of 3..10 ticks; negative means down, positive means up. "
                "Use Ctrl+End to reach a document's bottom if useful."
            )
        return [*messages, {"role": "user", "content": (
            "The LAST image is the current desktop. Earlier images are historical; "
            "verify outcomes only against the last image. " + instruction
        )}]


async def predict(agent, image: bytes, task: str) -> dict:
    from cua_agent.responses import make_input_image_item

    messages = [make_input_image_item(image), {"role": "user", "content": task}]
    messages = await agent._on_llm_start(messages)
    return await agent.agent_loop.predict_step(
        messages=messages, model=agent.model, api_base=agent.api_base, api_key=agent.api_key,
        tools=agent.tool_schemas, computer_handler=agent.computer_handler,
        _on_api_end=agent._on_api_end, max_retries=0, stream=False, **agent.kwargs,
    )


async def probe_model(agent, desktop: bytes) -> dict:
    from PIL import Image, ImageDraw

    from .results import response_text

    answer = await predict(agent, desktop, "Describe one visible feature of this screenshot. "
                           "Do not call tools; this is a read-only connection check.")
    if not response_text(answer).strip():
        raise RuntimeError("Model did not return a screenshot description")
    report = {"vision": "passed", "actions_executed": 0}
    if "extra_body" not in agent.kwargs:
        return report
    image = Image.new("RGB", (640, 384), "white")
    draw = ImageDraw.Draw(image)
    draw.rectangle((400, 240, 600, 340), fill="blue")
    draw.text((460, 280), "START", fill="white", font_size=28)
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    prediction = await predict(agent, buffer.getvalue(), "Click the blue START button. "
                               "Return only the next computer tool call.")
    calls = [i for i in prediction.get("output", []) if i.get("type") == "computer_call"]
    if len(calls) != 1:
        raise RuntimeError("Model did not predict exactly one computer action")
    action = calls[0].get("action", {})
    if (action.get("type") != "left_click" or not 400 <= action.get("x", -1) <= 600
            or not 240 <= action.get("y", -1) <= 340):
        raise RuntimeError("Model coordinates do not match Cua's screenshot convention")
    return {**report, "tool_format": "passed", "coordinates": "passed"}
