"""Inline the scene, section map and fonts into the viewer page."""
import base64
from pathlib import Path

d = Path(__file__).parent
page = (d / "viewer.template.html").read_text()
page = page.replace("__SCENE__", (d / "neomem.excalidraw").read_text())
page = page.replace("__SECTIONS__", (d / "sections.json").read_text())
for name in ("Virgil", "Cascadia"):
    page = page.replace(f"__{name.upper()}__", base64.b64encode((d / "fonts" / f"{name}.woff2").read_bytes()).decode())
(d / "neomem-board.html").write_text(page)
print("wrote", d / "neomem-board.html", len(page) // 1024, "KB")
