//! Weather from a host's provider: `weather in Tokyo`, `humidity in London tomorrow`.

use jiff::civil::Date;
use jiff::tz::TimeZone;

/// What a weather line asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherQuery {
    /// The temperature and the sky: "18 °C (partly cloudy)".
    Summary,
    Temperature,
    /// The day's highest temperature.
    High,
    /// The day's lowest temperature.
    Low,
    Humidity,
    WindSpeed,
    /// Where the wind comes from.
    WindDirection,
    UvIndex,
    ChanceOfRain,
    /// What feels like the temperature, with wind and humidity.
    FeelsLike,
    /// Rain, showers and snow as water.
    Precipitation,
    Rain,
    Snowfall,
    CloudCover,
    Visibility,
    DewPoint,
    /// Air pressure at sea level, and whether it is rising or falling.
    Pressure,
    /// The sky in words: "partly cloudy".
    Conditions,
}

/// Where and when the weather is asked for.
#[derive(Clone, Debug)]
pub struct WeatherRequest {
    /// The place as the line names it: "Tokyo".
    pub place: String,
    pub latitude: f64,
    pub longitude: f64,
    pub zone: TimeZone,
    /// A day, or `None` for now.
    pub date: Option<Date>,
}

/// The weather at a place, now or on a day; a provider leaves out what it doesn't know.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Weather {
    /// Degrees Celsius, now, or the middle of the day's range on another day.
    pub temperature: Option<f64>,
    /// The day's highest and lowest, in degrees Celsius.
    pub high: Option<f64>,
    pub low: Option<f64>,
    /// Relative humidity in percent.
    pub humidity: Option<f64>,
    /// Kilometers an hour.
    pub wind_speed: Option<f64>,
    /// Degrees from north, where the wind comes from.
    pub wind_direction: Option<f64>,
    pub uv_index: Option<f64>,
    /// Percent.
    pub chance_of_rain: Option<f64>,
    /// Degrees Celsius.
    pub feels_like: Option<f64>,
    /// Millimeters, in the last hour now or over the day; snow as water.
    pub precipitation: Option<f64>,
    pub rain: Option<f64>,
    /// Centimeters.
    pub snowfall: Option<f64>,
    /// Percent of the sky.
    pub cloud_cover: Option<f64>,
    /// Kilometers.
    pub visibility: Option<f64>,
    /// Degrees Celsius.
    pub dew_point: Option<f64>,
    /// Hectopascals at sea level.
    pub pressure: Option<f64>,
    /// How much the pressure changed over the last three hours, in hectopascals.
    pub pressure_change: Option<f64>,
    /// The sky in a few words: "partly cloudy".
    pub condition: Option<String>,
}

/// A source of weather, as SoulverCore's `WeatherDataProvider`.
pub trait WeatherProvider: Send + Sync {
    fn weather(&self, request: &WeatherRequest) -> Result<Weather, String>;

    /// The weather is being fetched in the background: the line answers `ErrorKind::Pending`.
    fn pending(&self, request: &WeatherRequest) -> bool {
        let _ = request;
        false
    }
}

/// The sky for a WMO weather code, as weather services report it.
pub fn condition(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "clear sky",
        1 => "mainly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "fog",
        51 | 53 | 55 => "drizzle",
        56 | 57 => "freezing drizzle",
        61 => "light rain",
        63 => "rain",
        65 => "heavy rain",
        66 | 67 => "freezing rain",
        71 => "light snow",
        73 => "snow",
        75 => "heavy snow",
        77 => "snow grains",
        80..=82 => "rain showers",
        85 | 86 => "snow showers",
        95 => "thunderstorm",
        96 | 99 => "thunderstorm with hail",
        _ => return None,
    })
}
