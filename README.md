# AI Movie Shorts Generator

An automated tool that transforms full-length movies into engaging short-form video content with AI-generated narration, subtitles, and background music. Generates both horizontal (YouTube) and vertical (TikTok/Reels) formats.

## Features

- **Automatic Subtitle Download**: Fetches English subtitles from subf2m.co
- **Script Enhancement**: Optionally fetches movie scripts from IMSDb for better context
- **AI-Powered Clip Selection**: Uses OpenAI to intelligently select key scenes and generate narration
- **Text-to-Speech**: Converts narration to speech using ElevenLabs
- **Background Music**: Automatically mixes in background music tracks
- **Dual Format Output**: Creates both horizontal (16:9) and vertical (9:16) videos
- **GUI & CLI**: Use the graphical interface or run from command line

## Requirements

- **Rust** 1.85 or later
- **FFmpeg** (must be installed and available in PATH)
- **OpenAI API Key**
- **ElevenLabs API Key**

## Installation

```bash
# Clone the repository
git clone <repository-url>
cd ai-movie-shorts

# Build the release version
cargo build --release

# Or build with GUI support (default)
cargo build --release

# Build CLI-only version
cargo build --release --no-default-features
```

## Configuration

Create a `config.json` file in the project root:

```json
{
  "open_api_key": "your-openai-api-key",
  "elevenlabs_api_key": "your-elevenlabs-api-key",
  "eleven_voice_id": "JBFqnCBsd6RMkjVDRZzb",
  "eleven_model_id": "eleven_multilingual_v2"
}
```

**Required fields:**
- `open_api_key`: Your OpenAI API key
- `elevenlabs_api_key`: Your ElevenLabs API key

**Optional fields:**
- `eleven_voice_id`: Voice ID for narration (default: `JBFqnCBsd6RMkjVDRZzb`)
- `eleven_model_id`: TTS model to use (default: `eleven_multilingual_v2`)

## Folder Structure

The tool expects the following directory structure:

```
ai-movie-shorts/
├── config.json              # API configuration
├── movies/                  # Place your movie files here (.mp4)
├── backgroundmusic/         # Background music tracks (.mp3 or .m4a)
├── output/                  # Generated horizontal videos
├── tiktok_output/           # Generated vertical videos
├── movies_retired/          # Processed movies are moved here
├── scripts/srt_files/       # Downloaded subtitles and scripts (auto-created)
├── clips/                   # Temporary clip files (auto-created)
└── resources/               # UI resources (for GUI mode)
    └── Inter-Regular.ttf
```

## Usage

### GUI Mode (Default)

```bash
# Run the GUI application
cargo run --release --bin ai-movie-shorts

# Or run the compiled binary
./target/release/ai-movie-shorts
```

The GUI provides:
- **Folder buttons**: Open movies, retired movies, output, and SRT folders
- **START GENERATION button**: Begin processing all movies in the `movies/` folder
- **Log panel**: Real-time processing logs

### CLI Mode

```bash
# Run in CLI mode
cargo run --release --bin ai-movie-cli

# Or run the compiled binary
./target/release/ai-movie-cli
```

The CLI processes all movies in the `movies/` folder and exits when complete.

## Workflow

1. **Place Movies**: Put your `.mp4` movie files in the `movies/` folder
2. **Add Background Music**: Add `.mp3` or `.m4a` files to `backgroundmusic/` (optional)
3. **Configure**: Set up your `config.json` with API keys
4. **Run**: Start the generator using GUI or CLI
5. **Processing**:
   - Downloads subtitles automatically (or uses existing `.srt` files)
   - Fetches movie scripts from IMSDb for context (optional)
   - AI analyzes content and creates a clip plan (20-30 clips, 2.5-4.5 min total)
   - Generates narration for each clip
   - Converts narration to speech using ElevenLabs
   - Extracts video clips and adjusts timing to match narration
   - Concatenates clips into final video
   - Mixes in background music
   - Creates vertical version for TikTok/Reels
6. **Output**: 
   - Horizontal video: `output/{movie_name}.mp4`
   - Vertical video: `tiktok_output/{movie_name}_vertical.mp4`
   - Original movie moved to: `movies_retired/`

## Manual Subtitle Override

If automatic subtitle download fails, you can manually add subtitle files:

1. Create the folder: `scripts/srt_files/`
2. Add your `.srt` file with the exact movie name: `{movie_name}.srt`
3. The tool will use your subtitle file instead of downloading

## Tips

- **Movie Naming**: Use clear movie titles for better subtitle/script matching
- **Background Music**: Add multiple tracks for variety; tracks under 60 seconds are skipped
- **Processing Time**: Depends on movie length, clip count, and API response times
- **Storage**: Ensure sufficient disk space for temporary clip files
- **Existing Outputs**: Movies with existing output files are automatically skipped

## Troubleshooting

**Issue: Subtitles not downloading**
- Check your internet connection
- Try manually placing the `.srt` file in `scripts/srt_files/`

**Issue: IMSDb script not found**
- This is optional; the tool continues with subtitles only
- Some movies may not be available on IMSDb

**Issue: FFmpeg errors**
- Ensure FFmpeg is installed: `ffmpeg -version`
- Check that FFmpeg is in your system PATH

**Issue: API errors**
- Verify your API keys in `config.json`
- Check your API quotas (OpenAI and ElevenLabs)

## License

[Your License Here]

## Credits

- Subtitles sourced from [subf2m.co](https://subf2m.co)
- Scripts sourced from [IMSDb](https://imsdb.com)
- TTS powered by [ElevenLabs](https://elevenlabs.io)
- AI powered by [OpenAI](https://openai.com)
