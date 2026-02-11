import pytest


@pytest.fixture
def sample_track_dict():
    return {
        "trackID": 12345,
        "songID": 678,
        "songTitle": "Tweezer",
        "trackNum": 1,
        "discNum": 1,
        "setNum": 1,
        "totalRunningTime": 960,
        "hhmmssTotalRunningTime": "16:00",
    }


@pytest.fixture
def sample_show_dict():
    return {
        "containerID": 99999,
        "artistName": "Phish",
        "containerInfo": "2024-08-31 Dick's Sporting Goods Park",
        "venueName": "Dick's Sporting Goods Park",
        "venueCity": "Commerce City",
        "venueState": "CO",
        "performanceDate": "2024-08-31",
        "performanceDateFormatted": "08/31/2024",
        "performanceDateYear": "2024",
        "totalContainerRunningTime": 11565,
        "hhmmssTotalRunningTime": "3:12:45",
        "tracks": [],
        "songs": [],
        "img": {"url": "https://example.com/img.jpg"},
    }


@pytest.fixture
def sample_catalog_show_dict():
    return {
        "containerID": 99999,
        "artistName": "Phish",
        "containerInfo": "2024-08-31 Dick's Sporting Goods Park",
        "venueName": "Dick's Sporting Goods Park",
        "venueCity": "Commerce City",
        "venueState": "CO",
        "performanceDate": "2024-08-31",
        "performanceDateFormatted": "08/31/2024",
        "performanceDateYear": "2024",
        "img": {"url": "https://example.com/img.jpg"},
        "songList": "Tweezer, Sand, Piper",
    }
