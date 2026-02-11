"""LivePhish API client ported from Go LivePhish-Downloader."""

from __future__ import annotations

import hashlib
import time
from typing import Any

import httpx

from dataclasses import asdict

from livephish.models import Show, StreamParams

# API Constants
CLIENT_ID = "Fujeij8d764ydxcnh4676scsr7f4"
DEVELOPER_KEY = "njeurd876frhdjxy6sxxe721"
SIG_KEY = "jdfirj8475jf_"
USER_AGENT = "LivePhish/3.4.5.357 (Android; 7.1.2; Asus; ASUS_Z01QD)"
API_BASE = "https://streamapi.livephish.com/"
AUTH_BASE = "https://id.livephish.com/connect/"
RATE_LIMIT_DELAY = 0.5
MAX_RETRIES = 3


class APIError(Exception):
    """Base exception for API errors."""
    pass


class AuthError(APIError):
    """Authentication failed."""
    pass


class SubscriptionError(APIError):
    """Subscription validation failed."""
    pass


class LivePhishAPI:
    """Client for LivePhish API with authentication and streaming."""

    def __init__(self) -> None:
        self._client = httpx.Client(
            headers={"User-Agent": USER_AGENT},
            timeout=30.0,
            follow_redirects=True,
        )
        self._access_token: str | None = None
        self._last_request_time: float = 0
        self._email: str | None = None
        self._password: str | None = None

    def _rate_limit(self) -> None:
        """Enforce rate limiting between requests."""
        elapsed = time.time() - self._last_request_time
        if elapsed < RATE_LIMIT_DELAY:
            time.sleep(RATE_LIMIT_DELAY - elapsed)

    def _request(
        self,
        method: str,
        url: str,
        *,
        _rate_limit: bool = True,
        _allow_reauth: bool = True,
        **kwargs: Any,
    ) -> httpx.Response:
        """Make HTTP request with retry logic, rate limiting, and re-auth."""
        if _rate_limit:
            self._rate_limit()

        for attempt in range(MAX_RETRIES):
            try:
                response = self._client.request(method, url, **kwargs)
                self._last_request_time = time.time()

                # Re-auth on 401 if credentials are available
                if (
                    response.status_code == 401
                    and _allow_reauth
                    and self._email
                    and self._password
                ):
                    self.login(self._email, self._password)
                    return self._request(
                        method, url, _rate_limit=_rate_limit, _allow_reauth=False, **kwargs
                    )

                return response
            except (httpx.TransportError, httpx.TimeoutException) as e:
                if attempt == MAX_RETRIES - 1:
                    raise APIError(f"Request failed after {MAX_RETRIES} retries: {e}")
                # Exponential backoff: 1s, 2s, 4s
                time.sleep(2 ** attempt)

        # Should never reach here, but satisfies type checker
        raise APIError("Unexpected retry logic failure")

    def authenticate(self, email: str, password: str) -> str:
        """
        Authenticate with OAuth2 and get access token.

        Args:
            email: User email address
            password: User password

        Returns:
            Access token string

        Raises:
            AuthError: If authentication fails
        """
        response = self._request(
            "POST",
            f"{AUTH_BASE}token",
            _rate_limit=False,
            _allow_reauth=False,
            data={
                "client_id": CLIENT_ID,
                "grant_type": "password",
                "scope": "offline_access nugsnet:api nugsnet:legacyapi",
                "username": email,
                "password": password,
            },
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )

        if response.status_code != 200:
            try:
                error_data = response.json()
                error_key = error_data.get("error", "")
            except Exception:
                error_key = ""
            messages = {
                "invalid_grant": "Invalid email or password",
                "invalid_client": "Service temporarily unavailable",
            }
            msg = messages.get(error_key, f"HTTP {response.status_code}")
            raise AuthError(msg)

        data = response.json()
        self._access_token = data["access_token"]
        return self._access_token

    def get_user_token(self, email: str, password: str) -> str:
        """
        Get legacy session token for API calls.

        Args:
            email: User email address
            password: User password

        Returns:
            Session token string
        """
        response = self._request(
            "GET",
            f"{API_BASE}secureApi.aspx",
            _rate_limit=False,
            _allow_reauth=False,
            params={
                "method": "session.getUserToken",
                "clientID": CLIENT_ID,
                "developerKey": DEVELOPER_KEY,
                "user": email,
                "pw": password,
            },
        )

        data = response.json()
        return data["Response"]["tokenValue"]

    def get_subscriber_info(
        self, email: str, session_token: str
    ) -> tuple[StreamParams, str]:
        """
        Get subscriber information and stream parameters.

        Args:
            email: User email address
            session_token: Session token from get_user_token

        Returns:
            Tuple of (StreamParams, plan_name)

        Raises:
            SubscriptionError: If subscription is not valid for streaming
        """
        headers = {}
        if self._access_token:
            headers["Authorization"] = f"Bearer {self._access_token}"

        response = self._request(
            "GET",
            f"{API_BASE}secureApi.aspx",
            _rate_limit=False,
            _allow_reauth=False,
            params={
                "method": "user.getSubscriberInfo",
                "developerKey": DEVELOPER_KEY,
                "user": email,
                "token": session_token,
            },
            headers=headers,
        )

        data = response.json()
        sub_info = data["Response"]["subscriptionInfo"]

        # Check if user can stream content
        if not sub_info.get("canStreamSubContent"):
            raise SubscriptionError("Subscription does not allow streaming content")

        # Parse stream parameters
        stream_params = StreamParams(
            subscription_id=str(sub_info["subscriptionID"]),
            sub_costplan_id_access_list=sub_info["subCostplanIDAccessList"],
            user_id=str(sub_info["userID"]),
            start_stamp=str(sub_info["startDateStamp"]),
            end_stamp=str(sub_info["endDateStamp"]),
        )

        plan_name = sub_info.get("planName", "Unknown Plan")

        return stream_params, plan_name

    def get_catalog_page(self, offset: int = 1, limit: int = 100) -> list[dict]:
        """
        Get a page of catalog containers (shows).

        Args:
            offset: Starting offset (1-indexed)
            limit: Number of containers to return

        Returns:
            List of raw container dicts
        """
        response = self._request(
            "GET",
            f"{API_BASE}api.aspx",
            params={
                "method": "catalog.containersAll",
                "limit": limit,
                "startOffset": offset,
                "vdisp": 1,
            },
        )

        data = response.json()
        return data["Response"]["containers"]

    def get_show_detail(self, container_id: int) -> Show:
        """
        Get detailed show information including tracks.

        Args:
            container_id: Container ID to fetch

        Returns:
            Show object with full details
        """
        response = self._request(
            "GET",
            f"{API_BASE}api.aspx",
            params={
                "method": "catalog.container",
                "containerID": container_id,
                "vdisp": 1,
            },
        )

        if response.status_code != 200:
            raise APIError(
                f"Failed to fetch show {container_id}: HTTP {response.status_code}"
            )
        try:
            data = response.json()
        except ValueError:
            raise APIError(
                f"Invalid API response for show {container_id}"
            )
        if "Response" not in data:
            raise APIError(
                f"Unexpected API response for show {container_id}"
            )
        return Show.from_dict(data["Response"])

    def get_stream_url(
        self,
        track_id: int,
        format_code: int,
        stream_params: StreamParams,
        epoch_compensation: int = 0,
    ) -> str:
        """
        Get streaming URL for a track.

        Args:
            track_id: Track ID to stream
            format_code: Format code (2=ALAC, 3=AAC, 4=FLAC)
            stream_params: Stream parameters from get_subscriber_info
            epoch_compensation: Optional time offset for signature

        Returns:
            Stream URL string
        """
        # Generate timestamp and signature
        timestamp = int(time.time()) + epoch_compensation
        timestamp_str = str(timestamp)
        sig_input = SIG_KEY + timestamp_str
        sig = hashlib.md5(sig_input.encode()).hexdigest()

        # Override User-Agent for this specific request
        headers = {"User-Agent": "LivePhishAndroid"}

        response = self._request(
            "GET",
            f"{API_BASE}bigriver/subPlayer.aspx",
            params={
                "trackID": track_id,
                "app": 1,
                "platformID": format_code,
                "subscriptionID": stream_params.subscription_id,
                "subCostplanIDAccessList": stream_params.sub_costplan_id_access_list,
                "nn_userID": stream_params.user_id,
                "startDateStamp": stream_params.start_stamp,
                "endDateStamp": stream_params.end_stamp,
                "tk": sig,
                "lxp": timestamp_str,
            },
            headers=headers,
        )

        data = response.json()
        return data.get("streamLink", "")

    def login(self, email: str, password: str) -> tuple[StreamParams, str]:
        """
        Complete login flow: authenticate, get token, and get subscriber info.

        Args:
            email: User email address
            password: User password

        Returns:
            Tuple of (StreamParams, plan_name)
        """
        self.authenticate(email, password)
        session_token = self.get_user_token(email, password)
        return self.get_subscriber_info(email, session_token)

    def login_cached(
        self, email: str, password: str
    ) -> tuple[StreamParams, str]:
        """Try cached session first, fall back to full login.

        Trusts cached tokens within TTL without a validation call.
        Expired tokens are handled lazily via re-auth in _request().

        Returns:
            Tuple of (StreamParams, status_message)
        """
        from livephish.config import (
            clear_session_cache,
            load_session_cache,
            save_session_cache,
        )

        # Store credentials for re-auth in _request()
        self._email = email
        self._password = password

        cached = load_session_cache()
        if cached:
            try:
                self._access_token = cached["access_token"]
                sp = cached["stream_params"]
                params = StreamParams(
                    subscription_id=sp["subscription_id"],
                    sub_costplan_id_access_list=sp["sub_costplan_id_access_list"],
                    user_id=sp["user_id"],
                    start_stamp=sp["start_stamp"],
                    end_stamp=sp["end_stamp"],
                )
                return params, "Cached session"
            except (KeyError, ValueError):
                clear_session_cache()

        # Full 3-step auth
        params, plan_name = self.login(email, password)
        save_session_cache(
            access_token=self._access_token or "",
            session_token="",  # not needed after login
            stream_params_dict=asdict(params),
        )
        return params, plan_name

    def close(self) -> None:
        """Close the HTTP client."""
        self._client.close()
