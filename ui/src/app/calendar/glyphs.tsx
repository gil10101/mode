// The two chip glyphs of 03 §6.3 / 10 §7.4.12 (Lucide `video` and
// `file-pen`, ISC): a join link exists, a prepare note exists. Inline so the
// kit's icon sheet stays as it is (kit candidates).
import { BrandMark, brandForConference } from '../../kit/brands'

/** The conference provider's mark (Meet, Zoom, Teams), or the video glyph for any other join link. */
export function JoinGlyph({ conference }: { conference?: string | null }) {
  const id = brandForConference(conference)
  return id ? <BrandMark id={id} size={12} className="cv-glyph" label="has a join link" /> : <VideoGlyph />
}

export function VideoGlyph() {
  return (
    <svg className="m-icon cv-glyph" viewBox="0 0 24 24" aria-label="has a join link" role="img">
      <path d="m16 13 5.2 3.5a.5.5 0 0 0 .8-.4V7.9a.5.5 0 0 0-.8-.4L16 10.5" />
      <rect x="2" y="6" width="14" height="12" rx="2" />
    </svg>
  )
}

export function NoteGlyph() {
  return (
    <svg className="m-icon cv-glyph" viewBox="0 0 24 24" aria-label="has a prepare note" role="img">
      <path d="M12.5 22H18a2 2 0 0 0 2-2V7l-5-5H6a2 2 0 0 0-2 2v9.5" />
      <path d="M14 2v4a2 2 0 0 0 2 2h4" />
      <path d="M13.4 15.6a1 1 0 1 0-3-3l-5 5a2 2 0 0 0-.5.9l-.8 2.9a.5.5 0 0 0 .6.6l2.9-.8a2 2 0 0 0 .9-.5z" />
    </svg>
  )
}
