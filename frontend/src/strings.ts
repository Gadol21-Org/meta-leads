/** Every piece of user-facing text in the app, in Hebrew. Names (Meta Leads) stay in English. */

export const LOCALE = "he-IL";

/** Hebrew plural: one form for exactly 1, another otherwise. `one` is used with no number ("ליד אחד"). */
const plural = (n: number, one: string, many: string): string => (n === 1 ? one : `${n} ${many}`);

export const leads = (n: number): string => plural(n, "ליד אחד", "לידים");
export const forms = (n: number): string => plural(n, "טופס אחד", "טפסים");
export const devices = (n: number): string => plural(n, "מכשיר אחד", "מכשירים");

export const t = {
  appName: "Meta Leads",
  shortName: "Leads",
  nav: { stats: "סטטיסטיקה", menu: "תפריט", backToLeads: "חזרה ללידים" },
  notifications: {
    enable: "הפעלת התראות",
    addToHomeScreen: "הוסיפו למסך הבית כדי לקבל התראות",
    unsupported: "התראות אינן נתמכות",
    on: "ההתראות פועלות במכשיר הזה.",
    notAllowed: "ההתראות לא אושרו.",
    off: "ההתראות כובו במכשיר הזה.",
    notSubscribed: "המכשיר הזה אינו רשום להתראות.",
    sentTo: (n: number): string => `נשלח אל ${devices(n)}.`,
  },
  menu: { testPush: "שליחת התראת בדיקה", unsubscribe: "כיבוי התראות במכשיר הזה" },
  list: {
    noToken: "אסימון העמוד של Meta לא הוגדר, ולכן הלידים מוצגים רק עם המזהה שלהם. ראו README.",
    empty: "אין לידים עדיין. לידים חדשים יופיעו כאן וכהתראות.",
  },
  lead: {
    adDetails: "פרטי המודעה",
    received: "התקבל",
    form: "טופס",
    campaign: "קמפיין",
    adSet: "קבוצת מודעות",
    ad: "מודעה",
    organic: "אורגני",
    partner: "שותף",
    leadId: "מזהה ליד",
    yes: "כן",
    no: "לא",
    checked: "סומן",
    notChecked: "לא סומן",
    retry: "ניסיון חוזר",
    whatsapp: "וואטסאפ",
    gone: "הליד הזה לא קיים (ייתכן שהוסר ב-Meta).",
  },
  state: {
    loading: "טוען…",
    wrong: (msg: string): string => `משהו השתבש: ${msg}`,
    couldNotStart: (msg: string): string => `לא ניתן להפעיל את האפליקציה: ${msg}`,
  },
  api: {
    sessionExpired: "פג תוקף ההתחברות. רעננו את הדף כדי להתחבר שוב.",
    badResponse: (status: number): string =>
      `השרת לא הגיב כראוי (HTTP ${status}). ייתכן שהוא מופעל מחדש או שהבקשה נמשכה זמן רב מדי; בדקו את יומני הקונטיינר.`,
  },
  sync: {
    changes: (added: number, removed: number): string => {
      const parts = [added > 0 && `${leads(added)} נוספו`, removed > 0 && `${leads(removed)} הוסרו`].filter(Boolean);
      return `סונכרן עם Meta: ${parts.join(", ")}.`;
    },
    upToDate: "הכול מעודכן מול Meta.",
    skippedForms: (n: number): string => `לא ניתן היה לקרוא במלואם ${forms(n)}, ולכן לא נבדקו בהם מחיקות.`,
    unverified: (n: number): string => `לא ניתן היה לאמת ${leads(n)}, והם נשמרו.`,
    failed: (msg: string): string => `הסנכרון עם Meta נכשל: ${msg}`,
  },
  time: {
    now: "הרגע",
    minutes: (n: number): string => `לפני ${n} דק׳`,
    hours: (n: number): string => `לפני ${n} שע׳`,
  },
  stats: {
    range: "טווח זמן",
    days: (n: number): string => `${n} ימים`,
    none: (days: number): string => `אין לידים ב-${days} הימים האחרונים.`,
    leads: "לידים",
    today: "היום",
    perDay: "ליום",
    onAverage: "בממוצע",
    busiestDay: "היום העמוס ביותר",
    peakHour: "שעת שיא",
    localTime: "שעון מקומי",
    paidOrganic: "ממומן / אורגני",
    vsBefore: (change: string, days: number): string => `${change} לעומת ${days} הימים שקדמו`,
    readout: (total: number, days: number): string => `${leads(total)} ב-${days} הימים האחרונים. הקישו על עמודה כדי לראות את היום שלה.`,
    hoursReadout: "מתי הלידים מגיעים, לפי השעון המקומי שלכם.",
    perDayTitle: "לידים לפי יום",
    byPlatform: "לפי פלטפורמה",
    byAd: "לפי מודעה",
    byCampaign: "לפי קמפיין",
    byForm: "לפי טופס",
    byHour: "לפי שעה ביום",
    to: (from: string, until: string): string => `${from} עד ${until}`,
    unknown: "לא ידוע",
    other: (n: number): string => `אחר (${n})`,
  },
} as const;
