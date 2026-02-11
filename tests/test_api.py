"""Tests for LivePhish API client using respx for HTTP mocking."""

import hashlib
import time
import pytest
import respx
import httpx
from livephish.api import (
    LivePhishAPI,
    AuthError,
    SubscriptionError,
    APIError,
    API_BASE,
    AUTH_BASE,
    SIG_KEY,
)
from livephish.models import StreamParams, Show


@pytest.fixture
def api():
    """Create and cleanup API client."""
    a = LivePhishAPI()
    yield a
    a.close()


@respx.mock
def test_authenticate_success(api):
    """Test successful OAuth2 authentication."""
    # Mock the token endpoint
    respx.post(f"{AUTH_BASE}token").mock(
        return_value=httpx.Response(200, json={"access_token": "test_token"})
    )

    token = api.authenticate("user@example.com", "password123")

    assert token == "test_token"
    assert api._access_token == "test_token"


@respx.mock
def test_authenticate_failure(api):
    """Test authentication failure with 401."""
    # Mock the token endpoint with 401
    respx.post(f"{AUTH_BASE}token").mock(
        return_value=httpx.Response(401, text="Unauthorized")
    )

    with pytest.raises(AuthError, match="HTTP 401"):
        api.authenticate("user@example.com", "wrong_password")


@respx.mock
def test_authenticate_invalid_grant(api):
    """Test authentication with invalid_grant error (wrong credentials)."""
    # Mock the token endpoint with 400 and invalid_grant error
    respx.post(f"{AUTH_BASE}token").mock(
        return_value=httpx.Response(400, json={"error": "invalid_grant"})
    )

    with pytest.raises(AuthError, match="Invalid email or password"):
        api.authenticate("user@example.com", "wrong_password")


@respx.mock
def test_authenticate_invalid_client(api):
    """Test authentication with invalid_client error (service issue)."""
    # Mock the token endpoint with 400 and invalid_client error
    respx.post(f"{AUTH_BASE}token").mock(
        return_value=httpx.Response(400, json={"error": "invalid_client"})
    )

    with pytest.raises(AuthError, match="Service temporarily unavailable"):
        api.authenticate("user@example.com", "password123")


@respx.mock
def test_get_user_token(api):
    """Test getting legacy session token."""
    # Mock the session token endpoint
    respx.get(f"{API_BASE}secureApi.aspx").mock(
        return_value=httpx.Response(
            200, json={"Response": {"tokenValue": "sess_token"}}
        )
    )

    token = api.get_user_token("user@example.com", "password123")

    assert token == "sess_token"


@respx.mock
def test_get_subscriber_info_success(api):
    """Test getting subscriber info with valid subscription."""
    # Set access token
    api._access_token = "test_access_token"

    # Mock the subscriber info endpoint
    respx.get(f"{API_BASE}secureApi.aspx").mock(
        return_value=httpx.Response(
            200,
            json={
                "Response": {
                    "subscriptionInfo": {
                        "subscriptionID": "sub_123",
                        "subCostplanIDAccessList": "1,2,3",
                        "userID": 456,
                        "startDateStamp": 1700000000,
                        "endDateStamp": 1800000000,
                        "canStreamSubContent": True,
                        "planName": "LivePhish+ Annual",
                    }
                }
            },
        )
    )

    stream_params, plan_name = api.get_subscriber_info("user@example.com", "sess_token")

    assert isinstance(stream_params, StreamParams)
    assert stream_params.subscription_id == "sub_123"
    assert stream_params.sub_costplan_id_access_list == "1,2,3"
    assert stream_params.user_id == "456"
    assert stream_params.start_stamp == "1700000000"
    assert stream_params.end_stamp == "1800000000"
    assert plan_name == "LivePhish+ Annual"


@respx.mock
def test_get_subscriber_info_no_subscription(api):
    """Test subscriber info with no streaming subscription."""
    # Mock the subscriber info endpoint with canStreamSubContent=false
    respx.get(f"{API_BASE}secureApi.aspx").mock(
        return_value=httpx.Response(
            200,
            json={
                "Response": {
                    "subscriptionInfo": {
                        "subscriptionID": "sub_123",
                        "subCostplanIDAccessList": "1,2,3",
                        "userID": 456,
                        "startDateStamp": 1700000000,
                        "endDateStamp": 1800000000,
                        "canStreamSubContent": False,
                        "planName": "Basic Plan",
                    }
                }
            },
        )
    )

    with pytest.raises(
        SubscriptionError, match="Subscription does not allow streaming content"
    ):
        api.get_subscriber_info("user@example.com", "sess_token")


@respx.mock
def test_get_catalog_page(api):
    """Test fetching catalog page."""
    # Mock the catalog endpoint
    respx.get(f"{API_BASE}api.aspx").mock(
        return_value=httpx.Response(
            200,
            json={
                "Response": {
                    "containers": [
                        {"containerID": 1, "artistName": "Phish"},
                        {"containerID": 2, "artistName": "Phish"},
                    ]
                }
            },
        )
    )

    containers = api.get_catalog_page(offset=1, limit=100)

    assert isinstance(containers, list)
    assert len(containers) == 2
    assert containers[0]["containerID"] == 1
    assert containers[1]["containerID"] == 2


@respx.mock
def test_get_show_detail(api):
    """Test fetching detailed show information."""
    # Mock the container detail endpoint
    respx.get(f"{API_BASE}api.aspx").mock(
        return_value=httpx.Response(
            200,
            json={
                "Response": {
                    "containerID": 12345,
                    "artistName": "Phish",
                    "containerInfo": "Madison Square Garden",
                    "venueName": "Madison Square Garden",
                    "venueCity": "New York",
                    "venueState": "NY",
                    "performanceDate": "2024-08-31",
                    "performanceDateFormatted": "August 31, 2024",
                    "performanceDateYear": "2024",
                    "tracks": [
                        {
                            "trackID": 1,
                            "songID": 100,
                            "songTitle": "Wilson",
                            "trackNum": 1,
                            "discNum": 1,
                            "setNum": 1,
                        }
                    ],
                }
            },
        )
    )

    show = api.get_show_detail(12345)

    assert isinstance(show, Show)
    assert show.container_id == 12345
    assert show.artist_name == "Phish"
    assert show.venue_name == "Madison Square Garden"
    assert len(show.tracks) == 1
    assert show.tracks[0].song_title == "Wilson"


@respx.mock
def test_get_stream_url_signature(api, monkeypatch):
    """Test stream URL request with correct signature."""
    # Monkeypatch time.time to return fixed value
    fixed_time = 1234567890
    monkeypatch.setattr(time, "time", lambda: fixed_time)

    # Calculate expected signature
    epoch_compensation = 100
    expected_timestamp = fixed_time + epoch_compensation
    sig_input = SIG_KEY + str(expected_timestamp)
    expected_sig = hashlib.md5(sig_input.encode()).hexdigest()

    # Track request params
    request_params = {}

    def capture_params(request):
        nonlocal request_params
        request_params = dict(request.url.params)
        return httpx.Response(200, json={"streamLink": "https://stream.url/track.m4a"})

    # Mock the stream URL endpoint
    respx.get(f"{API_BASE}bigriver/subPlayer.aspx").mock(side_effect=capture_params)

    stream_params = StreamParams(
        subscription_id="sub_123",
        sub_costplan_id_access_list="1,2,3",
        user_id="456",
        start_stamp="1700000000",
        end_stamp="1800000000",
    )

    url = api.get_stream_url(
        track_id=999, format_code=4, stream_params=stream_params, epoch_compensation=epoch_compensation
    )

    # Verify the signature and timestamp in request params
    assert request_params["tk"] == expected_sig
    assert request_params["lxp"] == str(expected_timestamp)
    assert url == "https://stream.url/track.m4a"


@respx.mock
def test_retry_on_network_error(api, monkeypatch):
    """Test retry logic on network errors."""
    # Monkeypatch time.sleep to avoid actual waiting
    sleep_calls = []
    monkeypatch.setattr(time, "sleep", lambda x: sleep_calls.append(x))

    # Mock first request to raise ConnectError, second succeeds
    call_count = 0

    def mock_response(request):
        nonlocal call_count
        call_count += 1
        if call_count == 1:
            raise httpx.ConnectError("Connection failed")
        return httpx.Response(
            200, json={"Response": {"containers": [{"containerID": 1}]}}
        )

    respx.get(f"{API_BASE}api.aspx").mock(side_effect=mock_response)

    # Should succeed on second attempt
    containers = api.get_catalog_page()

    assert len(containers) == 1
    assert call_count == 2
    # Verify exponential backoff (first retry after 2^0 = 1 second)
    assert 1 in sleep_calls


@respx.mock
def test_login_convenience(api):
    """Test the login convenience method."""
    # Mock the token endpoint
    respx.post(f"{AUTH_BASE}token").mock(
        return_value=httpx.Response(200, json={"access_token": "test_token"})
    )

    # Mock secureApi.aspx with a side_effect that routes by method param
    def route_secure_api(request):
        method = request.url.params.get("method", "")
        if method == "session.getUserToken":
            return httpx.Response(200, json={"Response": {"tokenValue": "sess_token"}})
        elif method == "user.getSubscriberInfo":
            return httpx.Response(
                200,
                json={
                    "Response": {
                        "subscriptionInfo": {
                            "subscriptionID": "sub_123",
                            "subCostplanIDAccessList": "1,2,3",
                            "userID": 456,
                            "startDateStamp": 1700000000,
                            "endDateStamp": 1800000000,
                            "canStreamSubContent": True,
                            "planName": "LivePhish+ Annual",
                        }
                    }
                },
            )
        return httpx.Response(404)

    respx.get(f"{API_BASE}secureApi.aspx").mock(side_effect=route_secure_api)

    stream_params, plan_name = api.login("user@example.com", "password123")

    assert isinstance(stream_params, StreamParams)
    assert plan_name == "LivePhish+ Annual"
    assert api._access_token == "test_token"


@respx.mock
def test_login_cached_cache_miss(api, monkeypatch):
    """Test login_cached performs full auth when no cache exists."""
    # Mock no cache
    monkeypatch.setattr("livephish.config.load_session_cache", lambda: None)

    saved = {}
    def mock_save(access_token, session_token, stream_params_dict):
        saved.update({"access_token": access_token, "stream_params": stream_params_dict})
    monkeypatch.setattr("livephish.config.save_session_cache", mock_save)

    # Mock the token endpoint
    respx.post(f"{AUTH_BASE}token").mock(
        return_value=httpx.Response(200, json={"access_token": "new_token"})
    )

    def route_secure_api(request):
        method = request.url.params.get("method", "")
        if method == "session.getUserToken":
            return httpx.Response(200, json={"Response": {"tokenValue": "sess"}})
        elif method == "user.getSubscriberInfo":
            return httpx.Response(200, json={
                "Response": {
                    "subscriptionInfo": {
                        "subscriptionID": "sub_1",
                        "subCostplanIDAccessList": "1",
                        "userID": 1,
                        "startDateStamp": 100,
                        "endDateStamp": 200,
                        "canStreamSubContent": True,
                        "planName": "Test Plan",
                    }
                }
            })
        return httpx.Response(404)

    respx.get(f"{API_BASE}secureApi.aspx").mock(side_effect=route_secure_api)

    params, status = api.login_cached("user@example.com", "pass")
    assert isinstance(params, StreamParams)
    assert status == "Test Plan"
    assert saved["access_token"] == "new_token"
    # Verify credentials stored for re-auth
    assert api._email == "user@example.com"
    assert api._password == "pass"


@respx.mock
def test_login_cached_cache_hit(api, monkeypatch):
    """Test login_cached uses valid cached session without validation call."""
    import time as time_mod

    cached_data = {
        "access_token": "cached_token",
        "session_token": "cached_sess",
        "stream_params": {
            "subscription_id": "sub_cached",
            "sub_costplan_id_access_list": "1,2",
            "user_id": "99",
            "start_stamp": "100",
            "end_stamp": "200",
        },
        "cached_at": time_mod.time(),
    }
    monkeypatch.setattr("livephish.config.load_session_cache", lambda: cached_data)

    # No API mocks needed — trusted cache should not make any API calls
    params, status = api.login_cached("user@example.com", "pass")
    assert status == "Cached session"
    assert params.subscription_id == "sub_cached"
    assert api._access_token == "cached_token"
    assert api._email == "user@example.com"


@respx.mock
def test_login_cached_corrupt_cache(api, monkeypatch):
    """Test login_cached falls back to full auth when cache has bad keys."""
    cached_data = {
        "access_token": "old_token",
        "stream_params": {
            "subscription_id": "sub_old",
            # Missing required keys
        },
    }
    monkeypatch.setattr("livephish.config.load_session_cache", lambda: cached_data)

    cleared = []
    monkeypatch.setattr("livephish.config.clear_session_cache", lambda: cleared.append(True))
    monkeypatch.setattr("livephish.config.save_session_cache", lambda *a, **kw: None)

    # Mock full auth flow
    respx.post(f"{AUTH_BASE}token").mock(
        return_value=httpx.Response(200, json={"access_token": "fresh_token"})
    )

    def route_secure_api(request):
        method = request.url.params.get("method", "")
        if method == "session.getUserToken":
            return httpx.Response(200, json={"Response": {"tokenValue": "new_sess"}})
        elif method == "user.getSubscriberInfo":
            return httpx.Response(200, json={
                "Response": {
                    "subscriptionInfo": {
                        "subscriptionID": "sub_new",
                        "subCostplanIDAccessList": "1",
                        "userID": 1,
                        "startDateStamp": 100,
                        "endDateStamp": 200,
                        "canStreamSubContent": True,
                        "planName": "New Plan",
                    }
                }
            })
        return httpx.Response(404)

    respx.get(f"{API_BASE}secureApi.aspx").mock(side_effect=route_secure_api)

    params, status = api.login_cached("user@example.com", "pass")
    assert status == "New Plan"
    assert len(cleared) == 1  # Cache was cleared


@respx.mock
def test_reauth_on_401(api, monkeypatch):
    """Test that _request() re-authenticates on 401 and retries."""
    monkeypatch.setattr(time, "sleep", lambda x: None)

    api._email = "user@example.com"
    api._password = "pass"
    api._access_token = "stale_token"

    call_count = {"api": 0, "auth": 0}

    # First API call returns 401, second (after re-auth) succeeds
    def mock_api(request):
        method = request.url.params.get("method", "")
        if method == "catalog.container":
            call_count["api"] += 1
            if call_count["api"] == 1:
                return httpx.Response(401, text="Unauthorized")
            return httpx.Response(200, json={
                "Response": {
                    "containerID": 1,
                    "artistName": "Phish",
                    "containerInfo": "Test",
                    "venueName": "Venue",
                    "venueCity": "City",
                    "venueState": "ST",
                    "performanceDate": "2024-01-01",
                    "performanceDateFormatted": "01/01/2024",
                    "performanceDateYear": "2024",
                    "tracks": [],
                }
            })
        return httpx.Response(404)

    def route_secure_api(request):
        method = request.url.params.get("method", "")
        if method == "session.getUserToken":
            return httpx.Response(200, json={"Response": {"tokenValue": "sess"}})
        elif method == "user.getSubscriberInfo":
            return httpx.Response(200, json={
                "Response": {
                    "subscriptionInfo": {
                        "subscriptionID": "sub_1",
                        "subCostplanIDAccessList": "1",
                        "userID": 1,
                        "startDateStamp": 100,
                        "endDateStamp": 200,
                        "canStreamSubContent": True,
                        "planName": "Reauth Plan",
                    }
                }
            })
        return httpx.Response(404)

    respx.get(f"{API_BASE}api.aspx").mock(side_effect=mock_api)
    respx.get(f"{API_BASE}secureApi.aspx").mock(side_effect=route_secure_api)
    respx.post(f"{AUTH_BASE}token").mock(
        return_value=httpx.Response(200, json={"access_token": "fresh_token"})
    )

    # This should trigger re-auth transparently
    show = api.get_show_detail(1)
    assert show.container_id == 1
    assert call_count["api"] == 2  # First 401, then success after re-auth
    assert api._access_token == "fresh_token"


@respx.mock
def test_reauth_not_triggered_without_credentials(api, monkeypatch):
    """Test that _request() does NOT re-auth when credentials aren't stored."""
    monkeypatch.setattr(time, "sleep", lambda x: None)

    # No credentials stored
    api._email = None
    api._password = None

    respx.get(f"{API_BASE}api.aspx").mock(
        return_value=httpx.Response(401, text="Unauthorized")
    )

    # Should return 401 response without re-auth attempt
    response = api._request("GET", f"{API_BASE}api.aspx", params={"method": "catalog.container", "containerID": 1, "vdisp": 1})
    assert response.status_code == 401
