use chrono::{NaiveDate, NaiveTime};
use serenity::{
    builder::CreateEmbed,
    client::Context,
    model::{
        id::GuildId,
        prelude::{
            component::ButtonStyle, ChannelId, ChannelType, Message, PermissionOverwrite,
            PermissionOverwriteType, ReactionType, RoleId,
        },
        Permissions,
    },
};
use songbird::tracks::{LoopState, PlayMode, TrackQueue};
use std::env;

use crate::modules::util::{cap_string, EmbedColor};

use super::TrackRequesterId;

const CHANNEL_NAME: &str = "🔊lorelei";

pub async fn ensure_channel_exists(ctx: &Context) {
    let guild_id = GuildId(env::var("TEST_GUILD").unwrap().parse().unwrap());
    let guild = ctx.cache.guild(guild_id).unwrap();

    let mut channel_exists = false;
    for (id, _) in guild.channels {
        let name = id.name(ctx).await;
        if name == Some(CHANNEL_NAME.to_string()) {
            channel_exists = true;
        }
    }

    if !channel_exists {
        let channel_handle = guild_id
            .create_channel(&ctx.http, |c| {
                c.name(CHANNEL_NAME)
                    .kind(ChannelType::Text)
                    .topic("Play your favorite bangers")
                    .permissions(vec![PermissionOverwrite {
                        allow: Permissions::VIEW_CHANNEL,
                        deny: Permissions::SEND_MESSAGES,
                        kind: PermissionOverwriteType::Role(RoleId(
                            *guild_id.as_u64(), // @everyone
                        )),
                    }])
            })
            .await;

        let mut status_embed = CreateEmbed::default();
        populate_with_idle_status(ctx, &mut status_embed);

        channel_handle
            .as_ref()
            .unwrap()
            .send_message(&ctx.http, |m| m.set_embed(status_embed))
            .await
            .ok();

        let mut queue_embed = CreateEmbed::default();
        populate_with_idle_queue(&mut queue_embed);

        channel_handle
            .unwrap()
            .send_message(&ctx.http, |m| m.set_embed(queue_embed))
            .await
            .ok();
    }
}

/// Get a handle to the channel displaying bot state
async fn get_status_channel(ctx: &Context) -> Option<ChannelId> {
    let guild_id = GuildId(env::var("TEST_GUILD").unwrap().parse().unwrap());
    let guild = ctx.cache.guild(guild_id)?;

    for (id, _) in guild.channels {
        if id.name(ctx).await == Some(CHANNEL_NAME.to_string()) {
            return Some(id);
        }
    }

    None
}

/// Get a handle to the message used to convey bot information
pub async fn get_status_message(ctx: &Context) -> Option<Message> {
    let channel_id = get_status_channel(ctx).await?;
    let message_id = env::var("STATUS_MESSAGE")
        .expect("Temporary value required for development")
        .parse::<u64>()
        .unwrap();

    ctx.http
        .get_message(channel_id.into(), message_id)
        .await
        .ok()
}

/// Get a handle to the message used to convey queue information
pub async fn get_queue_message(ctx: &Context) -> Option<Message> {
    let channel_id = get_status_channel(ctx).await?;
    let message_id = env::var("QUEUE_MESSAGE")
        .expect("Temporary value required for development")
        .parse::<u64>()
        .unwrap();

    ctx.http
        .get_message(channel_id.into(), message_id)
        .await
        .ok()
}

/// Set the status message to display information about the current track
pub async fn update_status(ctx: &Context, queue: &TrackQueue) {
    let mut status_message = get_status_message(ctx).await.unwrap();
    let mut status_embed = CreateEmbed::default();

    let mut queue_message = get_queue_message(ctx).await.unwrap();
    let mut queue_embed = CreateEmbed::default();

    let current_track = queue.current();
    let current_track = match current_track {
        Some(track) => track,
        None => {
            populate_with_idle_status(ctx, &mut status_embed);

            status_message
                .edit(&ctx.http, |m| m.set_embed(status_embed).components(|c| c))
                .await
                .ok();

            return;
        }
    };

    let state = current_track
        .get_info()
        .await
        .expect("TrackState should exist");

    let is_looping = !matches!(state.loops, LoopState::Finite(0));
    let is_playing = matches!(state.playing, PlayMode::Play);

    populate_with_current_status(ctx, queue, &mut status_embed).await;
    populate_with_current_queue(queue, &mut queue_embed);

    status_message
        .edit(&ctx.http, |m| {
            m.set_embed(status_embed).components(|c| {
                c.create_action_row(|r| {
                    r.create_button(|b| {
                        b.emoji(ReactionType::Unicode("🔂".to_string()))
                            .style(if is_looping {
                                ButtonStyle::Success
                            } else {
                                ButtonStyle::Danger
                            })
                            .custom_id(if is_looping { "loop_off" } else { "loop_on" })
                    })
                    .create_button(|b| {
                        b.emoji(ReactionType::Unicode(
                            (if is_playing { "⏸" } else { "▶️" }).to_string(),
                        ))
                        .style(ButtonStyle::Secondary)
                        .custom_id(if is_playing {
                            "pause"
                        } else {
                            "play"
                        })
                    })
                    .create_button(|b| {
                        b.emoji(ReactionType::Unicode("⏩".to_string()))
                            .style(ButtonStyle::Secondary)
                            .custom_id("skip")
                    })
                    .create_button(|b| {
                        b.emoji(ReactionType::Unicode("⏹".to_string()))
                            .style(ButtonStyle::Secondary)
                            .custom_id("stop")
                    })
                })
            })
        })
        .await
        .ok();

    queue_message
        .edit(&ctx.http, |m| m.set_embed(queue_embed))
        .await
        .ok();
}

/// Populate the provided `CreateEmbed` with the default message to be
/// displayed when no activity is performed
fn populate_with_idle_status<'a>(ctx: &Context, embed: &'a mut CreateEmbed) -> &'a mut CreateEmbed {
    let bot = ctx.cache.current_user();

    embed
        .color(EmbedColor::Success.hex())
        .title(&bot.name)
        .url("https://github.com/btoschek/lorelei")
        .description("Play your favorite songs right in Discord")
        .thumbnail(bot.face())
        .field("Play a song", "/play URL", false)
}

/// Populate the provided `CreateEmbed` with the current status of
/// the bot, ready to be displayed to the end-user
async fn populate_with_current_status<'a>(
    ctx: &Context,
    queue: &TrackQueue,
    embed: &'a mut CreateEmbed,
) -> &'a mut CreateEmbed {
    let current_track = queue.current();
    let current_track = match current_track {
        Some(track) => track,
        None => return populate_with_idle_status(ctx, embed),
    };

    let meta = current_track.metadata();

    embed
        .title("Listening to")
        .description(format!(
            "[{}]({})",
            meta.title.as_ref().unwrap_or(&"Untitled".to_string()),
            meta.source_url
                .as_ref()
                .expect("We have to stream from something")
        ))
        .color(EmbedColor::Success.hex())
        .thumbnail(meta.thumbnail.as_ref().unwrap_or(
            &"https://ak.picdn.net/shutterstock/videos/34370329/thumb/1.jpg".to_string(),
        ));

    if let Some(artist) = &meta.artist.as_ref() {
        embed.footer(|f| f.text(artist));
    }

    if let Some(duration) = &meta.duration.as_ref() {
        let time = NaiveTime::from_num_seconds_from_midnight_opt(duration.as_secs() as u32, 0)
            .expect("Just crash if someone is trolling with lengths exceeding the heat death of the universe");

        embed.field("Duration", time.format("%H:%M:%S"), true);
    }

    if let Some(date) = &meta.date.as_ref() {
        let datetime = NaiveDate::parse_from_str(date, "%Y%m%d")
            .expect("Date format theoretically should not change");

        embed.field("Uploaded", datetime.format("%d.%m.%Y"), true);
    }

    let user = current_track.typemap().read().await;
    let user = user.get::<TrackRequesterId>();
    let user = user
        .unwrap()
        .to_user(&ctx)
        .await
        .expect("User has to exist");

    embed.field(
        "Queued by",
        if user.discriminator != 0 {
            format!("{}#{}", user.name, user.discriminator)
        } else {
            user.name
        },
        true,
    )
}

/// Populate the provided `CreateEmbed` with the default message to be
/// displayed when no tracks are queued (either only one track is playing or none)
fn populate_with_idle_queue(embed: &mut CreateEmbed) -> &mut CreateEmbed {
    embed
        .title("Upcoming tracks")
        .color(EmbedColor::Success.hex())
        .description("There are currently no tracks queued.")
}

/// Populate the provided `CreateEmbed` with information about the
/// current audio queue of the bot, ready to be displayed to the end-user
fn populate_with_current_queue<'a>(
    queue: &TrackQueue,
    embed: &'a mut CreateEmbed,
) -> &'a mut CreateEmbed {
    let numbers = [":one:", ":two:", ":three:", ":four:", ":five:"];
    let queue = queue.current_queue();

    if queue.len() <= 1 {
        return populate_with_idle_queue(embed);
    }

    // Format the next n items into a markdown string following the scheme:
    //   :one: [<Track title>](<Track URL>)
    //   :two: [<Track title>](<Track URL>)
    let mut embed_description = queue
        .iter()
        .skip(1)
        .take(numbers.len())
        .enumerate()
        .map(|(i, t)| {
            let meta = t.metadata();
            let title = meta
                .title
                .to_owned()
                .unwrap_or("Untitled".to_string())
                .replace('[', "(")
                .replace(']', ")");

            format!(
                "{} [{}]({})",
                numbers[i],
                cap_string(&title, 50),
                meta.source_url.as_ref().unwrap()
            )
        })
        .collect::<Vec<String>>()
        .join("\n");

    // Calculate the amount of tracks not being displayed
    let num_tracks_not_displayed = queue.len() as i64 - (numbers.len() as i64 + 1);

    // If some tracks can't be displayed, append a counter
    // with the amount of tracks not being shown
    if num_tracks_not_displayed > 0 {
        embed_description.push_str(&format!("\n\nand {} more", num_tracks_not_displayed));
    }

    embed
        .title("Upcoming tracks")
        .description(embed_description)
        .color(EmbedColor::Success.hex())
}
