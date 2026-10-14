/**
 * The weather in the top bar: an icon, the temperature, and the city when
 * there is room. Hidden while the sense is off or nothing is read yet. A
 * click opens Settings → Weather. Reads the same view the sky does.
 */
import React from 'react';
import { Cloud, CloudFog, CloudLightning, CloudRain, CloudSnow, Moon, Sun, type LucideIcon } from 'lucide-react';
import { useSkyStore, useWeatherPoll } from '../stores/skyStore';
import { cn } from '../lib/cn';

const ICONS: Record<string, LucideIcon> = {
  cloudy: Cloud, fog: CloudFog, rain: CloudRain, snow: CloudSnow, storm: CloudLightning,
};
const WORDS: Record<string, string> = {
  clear: 'Clear', cloudy: 'Cloudy', fog: 'Fog', rain: 'Rain', snow: 'Snow', storm: 'Storm',
};

export const WeatherChip: React.FC<{ onOpen?: () => void; className?: string }> = ({ onOpen, className }) => {
  useWeatherPoll();
  const view = useSkyStore((s) => s.view);
  const reading = view?.reading;
  if (!view || view.off || !reading) return null;
  const Icon = ICONS[reading.kind] ?? (reading.is_day ? Sun : Moon);
  const city = view.city?.name ?? reading.city;
  const title = `${city} · ${WORDS[reading.kind] ?? reading.kind} · ${view.auto ? 'automatic' : 'typed'}`;
  return (
    <button
      type="button"
      onClick={onOpen}
      title={title}
      className={cn('flex items-center gap-1 text-[11px] font-medium text-slate-500 dark:text-slate-400 hover:text-blue-500 transition-colors', className)}
    >
      <Icon size={14} />
      <span>{Math.round(reading.temp_c)}°</span>
      <span className="hidden md:inline max-w-[9rem] truncate">{city}</span>
    </button>
  );
};
