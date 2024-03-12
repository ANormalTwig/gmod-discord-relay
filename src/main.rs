use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use serenity::all::ChannelId;
use serenity::builder::CreateAllowedMentions;
use serenity::builder::CreateEmbed;
use serenity::builder::CreateEmbedFooter;
use serenity::builder::CreateMessage;
use serenity::model::Color;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixListener;

use serde::Deserialize;

use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;

mod byte_helper;
use byte_helper::ByteReading;

#[derive(Deserialize)]
struct BotConfig {
	token: String,
	steam_key: String,
	relays: HashMap<u64, String>,
}

struct Handler {
	config: BotConfig,
	relays: Arc<Mutex<HashMap<ChannelId, OwnedWriteHalf>>>,
}

#[async_trait]
impl EventHandler for Handler {
	async fn ready(&self, ctx: Context, ready: Ready) {
		println!("{} is connected!", ready.user.name);

		let ctx = Arc::new(ctx);

		for (channel, relay) in self.config.relays.iter() {
			let path = format!("/tmp/{relay}");

			match fs::remove_file(&path) {
				Ok(_) => (),
				Err(_) => {
					println!("Couldn't remove file '{path}', assuming it wasn't there.");
				}
			};

			let listener = match UnixListener::bind(&path) {
				Ok(v) => v,
				Err(e) => {
					eprintln!("Couldn't bind to '{path}', skipping relay... {e}");
					continue;
				}
			};

			let perms = fs::Permissions::from_mode(0o777);
			match fs::set_permissions(&path, perms) {
				Ok(()) => (),
				Err(_) => {
					eprintln!("failed to set socket permission");
					continue;
				}
			};

			let ctx = ctx.clone();
			let channel = ChannelId::new(*channel);
			let relays = self.relays.clone();

			let key = self.config.steam_key.clone();
			tokio::spawn(async move {
				loop {
					let (stream, _sockaddr) = match listener.accept().await {
						Ok(v) => v,
						Err(e) => {
							eprintln!("Failed to accept client, {}", e);
							return;
						}
					};
					println!("Accepted client");

					let embed = CreateEmbed::new()
						.color(Color::from_rgb(0, 255, 0))
						.title("🟢 Server Online");

					let builder = CreateMessage::new().embed(embed);

					let _ = channel.send_message(&ctx, builder).await;

					let mut relays = relays.lock().await;

					match relays.remove(&channel) {
						Some(mut stream) => {
							let _ = stream.shutdown();
						}
						None => (),
					};

					let (read, write) = stream.into_split();

					relays.insert(channel, write);

					let ctx = ctx.clone();
					let key = key.clone();
					tokio::spawn(async move {
						relay_handler(ctx, channel, read, key).await;
					});
				}
			});
		}
	}

	async fn message(&self, ctx: Context, msg: Message) {
		if msg.author.bot {
			return;
		}

		let mut relays = self.relays.lock().await;

		let stream: &mut OwnedWriteHalf = match relays.get_mut(&msg.channel_id) {
			Some(v) => v,
			None => return,
		};

		let clr = match &msg.member(&ctx).await {
			Ok(member) => match member.roles(&ctx) {
				Some(mut roles) => {
					roles.sort_by(|a, b| b.position.cmp(&a.position));
					match roles.get(0) {
						Some(role) => {
							if role.colour.0 == 0 {
								Color::from_rgb(255, 255, 255)
							} else {
								role.colour
							}
						}
						None => Color::from_rgb(255, 255, 255),
					}
				}
				None => Color::from_rgb(255, 255, 255),
			},
			Err(_) => Color::from_rgb(255, 255, 255),
		};
		let cbuf = vec![clr.r(), clr.g(), clr.b()];
		let data = format!(
			"{}\0{}\0",
			msg.author.global_name.unwrap_or(msg.author.name),
			msg.content
		);
		let _ = stream
			.write(&[cbuf, (&data.as_bytes()).to_vec()].concat())
			.await;
	}
}

mod steam;
use steam::SteamAPI;

async fn relay_handler(
	ctx: Arc<Context>,
	channel: ChannelId,
	mut stream: OwnedReadHalf,
	steam_key: String,
) {
	let steam_client = SteamAPI::new(steam_key);

	let mut buf = vec![0; 4096];
	loop {
		let n = match stream.read(&mut buf).await {
			Ok(n) => n,
			Err(e) => {
				eprintln!("Error reading a socket, {e}");
				0
			}
		};

		if n == 0 {
			println!("Read 0 bytes, stopping read.");

			let embed = CreateEmbed::new()
				.color(Color::from_rgb(255, 0, 0))
				.title("🔴 Server Offline");

			let builder = CreateMessage::new().embed(embed);

			let _ = channel.send_message(&ctx, builder).await;

			return;
		} else if n < 2 {
			println!("Received packet with invalid size...");
			continue;
		}

		let bytes = &buf[..n];

		match bytes[0] {
			// Message
			1 => {
				if bytes.len() < 4 { continue }

				let mut i: usize = 0;
				for b in &bytes[1..] {
					i += 1;
					if *b == 0 { break }
				}

				let name = match String::from_utf8(bytes[1..i].to_vec()) {
					Ok(v) => v,
					Err(_) => continue,
				};

				let content = match String::from_utf8(bytes[i + 1..].to_vec()) {
					Ok(v) => v,
					Err(_) => continue,
				};

				let builder = CreateMessage::new()
					.allowed_mentions(CreateAllowedMentions::new())
					.content(format!("**{name}**: {content}"));

				let msg = channel.send_message(ctx.clone(), builder).await;
				if let Err(why) = msg {
					eprintln!("Error when sending message, {why}");
				}
			},
			// Join / Leave
			2 => {
				if bytes.len() < 8 { continue }

				let (r, g, b) = (bytes[2], bytes[3], bytes[4]);
				let (player_count, max_players) = (bytes[5], bytes[6]);

				let (name, len) = match bytes.read_string(7) {
					Ok(v) => v,
					Err(_) => continue,
				};
				let (steamid, len2) = match bytes.read_string(7 + len) {
					Ok(v) => v,
					Err(_) => continue,
				};

				let embed = CreateEmbed::new()
					.description(steamid)
					.color(Color::from_rgb(r, g, b));

				let embed = match bytes[1] {
					1 => embed.title(format!("**{name}** is connecting..."))
						.footer(CreateEmbedFooter::new(format!("Players: ({player_count}+1/{max_players})"))),
					2 => {
						let (steamid64, len3) = match bytes.read_string(7 + len + len2) {
							Ok(v) => v,
							Err(_) => continue,
						};

						let embed = match steam_client.get_player_summaries(&steamid64).await {
							Ok(summary) => embed.thumbnail(&summary.response.players[0].avatarmedium),
							Err(_) => continue,
						};

						let foot = match bytes.read_string(7 + len + len2 + len3) {
							Ok((map, _)) => format!("Players: ({player_count}/{max_players}) | Map: {map}"),
							Err(_) => format!("Players: ({player_count}/{max_players})"),
						};

						embed.title(format!("**{name}** has joined the server."))
							.url(format!("https://steamcommunity.com/profiles/{steamid64}"))
							.footer(CreateEmbedFooter::new(foot))
					},
					3 => {
						let (reason, _) = match bytes.read_string(7 + len + len2) {
							Ok(v) => v,
							Err(_) => continue,
						};

						embed.title(format!("**{name}** has disconnected. ({reason})"))
							.footer(CreateEmbedFooter::new(format!("Players: ({player_count}/{max_players})")))
					},
					_ => continue,
				};

				let builder = CreateMessage::new()
					.allowed_mentions(CreateAllowedMentions::new())
					.embed(embed);

				let msg = channel.send_message(ctx.clone(), builder).await;
				if let Err(why) = msg {
					eprintln!("Error when sending message, {why}");
				}
			},
			// Map Change
			3 => {
				let (map, _) = match bytes.read_string(1) {
					Ok(v) => v,
					Err(_) => continue,
				};

				let embed = CreateEmbed::new()
					.color(Color::from_rgb(198, 156, 109))
					.title(format!("Server is changing map to '{map}'"));

				let builder = CreateMessage::new()
					.embed(embed);

				let _ = channel.send_message(&ctx, builder).await;

				return
			},
			// Status Command
			4 => todo!("Status command, possibly appending interactions to a queue and completing them here when the data is relayed."),
			_ => (),
		}
	}
}

#[tokio::main]
async fn main() {
	let bytes = fs::read("config.json").expect("Failed to read config.json");
	let data = String::from_utf8(bytes).expect("Invalid UTF-8");
	let config = serde_json::from_str::<BotConfig>(&data).expect("Failed to parse config.json");

	let relays = Arc::new(Mutex::new(HashMap::new()));

	let intents = GatewayIntents::GUILD_MEMBERS
		| GatewayIntents::GUILDS
		| GatewayIntents::GUILD_MESSAGES
		| GatewayIntents::MESSAGE_CONTENT;

	let mut client = Client::builder(&config.token, intents)
		.event_handler(Handler { config, relays })
		.await
		.expect("Error creating client");

	if let Err(why) = client.start().await {
		println!("Client error: {why}");
	}
}
