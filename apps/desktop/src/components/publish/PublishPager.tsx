import { ChevronLeft, ChevronRight } from "lucide-react";

export function PublishPager({
  page,
  size,
  total,
  onPage,
  label,
  dragPaging = false,
}: {
  page: number;
  size: number;
  total: number;
  onPage: (page: number) => void;
  label: string;
  dragPaging?: boolean;
}) {
  const pages = Math.max(1, Math.ceil(total / size)),
    current = Math.min(page, pages - 1);
  return (
    <div className="publish-pager">
      <span>
        {total
          ? `${current * size + 1}–${Math.min((current + 1) * size, total)} / ${total}`
          : "0 kết quả"}
      </span>
      <div>
        <button
          type="button"
          className="ghost icon-only"
          aria-label={`${label}: trang trước`}
          data-pw-device-page={
            dragPaging && current > 0 ? current - 1 : undefined
          }
          disabled={!current}
          onClick={() => onPage(current - 1)}
        >
          <ChevronLeft size={16} />
        </button>
        <span>
          {current + 1} / {pages}
        </span>
        <button
          type="button"
          className="ghost icon-only"
          aria-label={`${label}: trang tiếp`}
          data-pw-device-page={
            dragPaging && current + 1 < pages ? current + 1 : undefined
          }
          disabled={current + 1 >= pages}
          onClick={() => onPage(current + 1)}
        >
          <ChevronRight size={16} />
        </button>
      </div>
    </div>
  );
}
