import json
from pathlib import Path

import fhr_monitor_analyzer as fhr


CSV_TEXT = """Date,HR1,HRM,TOCO
2026-04-30 19:27:08.099,130,98,12
2026-04-30 19:27:09.099,132,99,14
2026-04-30 19:27:10.099,134,100,15
"""


def test_analyze_csv_returns_json_string():
    report = json.loads(fhr.analyze_csv(CSV_TEXT))

    assert report["channel"] == "HR1"
    assert report["analysis_mode"] == "chunk"
    assert report["input"]["rows"] == 3
    assert "windows" in report


def test_analyze_json_returns_json_string():
    request = {
        "episode_id": "episode-1",
        "sent_at": "2026-05-12T12:22:35.052Z",
        "analysis_options": {"fetal_channel": "HR1"},
        "samples": [
            {"t": "2026-05-12T11:52:35.052Z", "hr1": 130, "hrm": 98, "toco": 12},
            {"t": "2026-05-12T11:52:36.052Z", "hr1": 132, "hrm": 99, "toco": 14},
        ],
    }

    report = json.loads(fhr.analyze_json(json.dumps(request)))

    assert report["channel"] == "HR1"
    assert report["input"]["rows"] == 2


def test_file_entry_points_return_json(tmp_path: Path):
    csv_path = tmp_path / "sample.csv"
    csv_path.write_text(CSV_TEXT)
    json_path = tmp_path / "request.json"
    json_path.write_text(
        json.dumps(
            {
                "episode_id": "episode-1",
                "sent_at": "2026-05-12T12:22:35.052Z",
                "samples": [
                    {"t": "2026-05-12T11:52:35.052Z", "hr1": 130},
                ],
            }
        )
    )

    csv_report = json.loads(fhr.analyze_csv_file(csv_path))
    json_report = json.loads(fhr.analyze_json_file(json_path))

    assert csv_report["input"]["rows"] == 3
    assert json_report["input"]["rows"] == 1


def test_plot_csv_file_writes_png(tmp_path: Path):
    csv_path = tmp_path / "sample.csv"
    csv_path.write_text(CSV_TEXT)
    output = tmp_path / "plot.png"

    written = fhr.plot_csv_file(csv_path, output=output)

    assert written == str(output)
    assert output.exists()
    assert output.stat().st_size > 0
