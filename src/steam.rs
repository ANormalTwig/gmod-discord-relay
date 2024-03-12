use reqwest::Client;
use serde::Deserialize;
use std::{error::Error, fmt};

#[derive(Debug)]
pub struct SteamAPIError;

impl Error for SteamAPIError {}

impl fmt::Display for SteamAPIError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Error during request to Steam API")
	}
}

#[derive(Deserialize)]
pub struct SteamPlayerSummary {
	pub avatarmedium: String,
}

#[derive(Deserialize)]
pub struct SteamSummaryResponse {
	pub players: Vec<SteamPlayerSummary>,
}

#[derive(Deserialize)]
pub struct SteamSummary {
	pub response: SteamSummaryResponse,
}

pub struct SteamAPI {
	http_client: Client,
	steam_key: String,
}

impl SteamAPI {
	pub fn new(key: String) -> SteamAPI {
		return SteamAPI {
			http_client: Client::new(),
			steam_key: key,
		};
	}

	pub async fn get_player_summaries(
		&self,
		steamid64: &String,
	) -> Result<SteamSummary, SteamAPIError> {
		let url = format!("https://api.steampowered.com/ISteamUser/GetPlayerSummaries/v0002/?key={}&steamids={steamid64}", self.steam_key);

		match self.http_client.get(url).send().await {
			Ok(res) => match res.text().await {
				Ok(text) => match serde_json::from_str::<SteamSummary>(&text) {
					Ok(v) => Ok(v),
					Err(_) => Err(SteamAPIError),
				},
				Err(_) => Err(SteamAPIError),
			},
			Err(_) => Err(SteamAPIError),
		}
	}
}
