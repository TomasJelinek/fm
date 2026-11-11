# Midnight Explorer — dvoupanelový souborový manažer v Rustu

Datum: 4. listopadu 2026; doplněno 9. listopadu 2026
Aktualizace 11. listopadu: [plynulé pixelové posouvání](../../developer/SCROLLING.md)
platí pro panely, seznamy v Nastavení, Find, Jobs, nabídky i viewery. Kolečko má
krátký přerušitelný dojezd; přesné touchpadové vstupy zůstávají nativní. Panely
se zarovnávají na řádek až po skončení pohybu. Podmínkou ověření je kromě
správných souřadnic také měření snímků a kontrola skutečně vykreslených pixelů;
backend nadále načítá pouze omezené části dokumentů.

Přetahování souborů rozšiřuje [návrh folder-drop](2026-11-10-folder-drag-and-native-handoff.md):
cílem může být složka přímo v panelu i zachycená nativní větev stromu. Výchozí
akce podle Průzkumníka je přesun na stejném svazku, jinak kopie; Ctrl vynutí
kopii a Shift přesun. Zjištění svazku i veškeré operace provádí backend.

Aktualizace 10. listopadu: [ověřování přenosů a nativní Move](2026-11-10-transfer-verification-and-native-move.md).
V Nastavení → General je výchozí úplná kontrola SHA-256; po výslovném vypnutí
se kontroluje délka a první/prostřední/poslední blok po 64 KiB. Soubory do 192 KiB
se porovnají celé. Každá úloha si volbu uchová, včetně přenosů ze schránky,
drag/drop, stromu a archivu. Přesun na stejném svazku používá nativní rename bez
čtení či hashování obsahu. Toto nahrazuje starší bezvýjimečné požadavky SHA-256
a kopírování při každém Move níže. Grafy uchovávají průběh celé operace;
nativní rename se měří v položkách/s, nikoli ve vymyšlených přenesených bajtech.

Navazující [Make File List](2026-11-09-make-file-list-design.md) přidává export
výběru podle Salamanderu přes Ctrl+M: šablony, historii, proměnné, schránku,
interní viewer a ověřené uložení/připojení k souboru. Práce běží v backendu nad
neměnným výběrem; pozdější navigace nemění exportované položky.

[Asociace viewerů](2026-11-09-viewer-associations.md) jsou uživatelské nastavení
podle ID pluginu: vlastní přípony, návrat k manifestu a pořadí pro konflikty.
Platí pro textové i izolované obrazové/PDF viewery; Alt+F3 zachovává Basic.

Markdown počítá rozpočet stránky podle sémantických řádků, nejen znaků LF.
Zobrazení a index bezpečných bodů používají jeden průchod parserem. Každá neměnná
relace uchovává nejvýše 16 posledních stránek a 1 MiB jejich sémantických dat,
takže návrat na uloženou stránku nečte a neparsuje zdroj znovu. Vzdálený skok bez
známého kontextu stále používá omezený doslovný náhled; nejde o úplný index souboru.
GitHub styl má vlastní rozvržení bloků: neutrální nadpisy, pravidla pod H1/H2,
podbarvené bloky kódu, odsazené citace a seznamy a tabulky se střídáním řádků.
Volba uživatelského písma zůstává zachována. Compact/Comfortable a textové viewery
nepřebírají tyto dekorace.

Stav: průběžný návrh produktu Midnight Explorer 0.2. Aktuální rozšíření operací,
ovládání a pluginů níže mají přednost před historickými etapami tohoto dokumentu.
Pořadí přijatých navazujících implementací zachycuje
[plán etap](../plans/2026-11-05-follow-up-stages.md); samotný návrh neznamená hotovou funkci.

Nejnovější závazné [úpravy panelů, viewerů a dialogů](2026-11-07-panel-viewer-followups.md)
zahrnují Hot Paths uvnitř archivů (`archiv.zip@cesta`), viditelný filtr panelu,
Find s Masks/Regex/Expression a jednořádkovými výsledky, známé velikosti složek
bez hlášky „unknown“, podrobné informace o disku, Markdown GitHub/Compact/Comfortable
a čísla řádků s pozicí ve source vieweru. Výslovně ruší režim „Move with recovery“
a požadují skutečná přesunutelná/resizovatelná okna dialogů mimo hlavní okno.
[Stav funkcí](../../FEATURE_STATUS.md) a [backlog](../../IMPLEMENTATION_BACKLOG.md)
oddělují hotové funkce od rozpracovaných oprav a dosud neověřeného nasazení.

[Společná horní lišta](2026-11-08-shared-toolbar-design.md) nahrazuje dvě stejné
panelové lišty a používá kontext aktivního panelu. Obsahuje také kontrolní součty,
velikosti složek a vlastnosti; disky, cesty a karty zůstávají u každého panelu.
Rozbalené nabídky zachycují panel a generaci umístění, aby přepnutí nemohlo změnit
cíl již zahájeného vstupu. Úzká okna lištu posouvají vodorovně. Při nedostatku
výšky jsou vlastní příkazy dostupné z tlačítka ve společné liště.

Navazující [požadavky na providery, výkon a použitelnost](../plans/2026-11-09-provider-performance-usability.md)
mají vlastní průběžný seznam ověření. Filtr patří do každého panelu bezprostředně
vlevo od View, Network Neighborhood do jeho lišty disků. Zkrácené cesty musí
viditelně obsahovat tři tečky `...`; plná hodnota zůstává autoritou pro operace.
Chybu v panelu lze zavřít nebo kompletně zkopírovat přes backend do schránky.

Procházení archivu samo nerozbaluje soubory. Až výslovné čtení obsahu pro viewer
nebo kopírování může vyžadovat rozbalení člena. Obecné diskové odkládání ukládá
ověřený obsah do soukromého dočasného souboru, ze kterého lze číst rozsahy;
nevyžaduje celý rozbalený soubor v RAM ani zvláštní JPEG implementaci.
Aktuální [návrh streamování archivů](2026-11-09-archive-streaming-design.md)
ruší pevné limity velikosti souboru při zachování integrity, rušení operace,
omezené pracovní paměti a kontroly chyb zápisu včetně zaplněného disku.

Velikosti složek v archivu počítá backend pouze z metadat: Ctrl+Shift+S měří
vybrané složky, nebo všechny bezprostřední složky, pokud výběr neexistuje.
Výběr obsahující pouze soubory nezpůsobí výpočet všech složek. Space měří
aktuální složku a posune kurzor. Změna řazení nebo procházení stejného archivu
výpočet neruší; výsledky mají identitu mountu a položky, nikoli dočasnou cestu.
Panel zobrazuje průběh nejvýše čtyřikrát za sekundu a stavový řádek přidává známé
součty vybraných složek. Změna zdrojového archivu výsledky zneplatní.
Výběr rozbalených větví stromu vyžaduje samostatné napojení stromových identit;
dokončené napojení plochého seznamu tuto funkci samo nezajišťuje.

Porovnání souborů bez výběru v protějším panelu vyhledá stejný přesný nativní
název v jeho viditelném seznamu. Výslovně označený protějšek má přednost;
nesouvisející soubor pod kurzorem není náhradním kandidátem.

Aktuální [ovládání source a Markdown vieweru](../../developer/VIEWER_PRESENTATION.md)
zahrnuje výchozí číslování řádků, skutečnou logickou pozici, písma a profily
GitHub/Compact/Comfortable. Indexování vzdálených pozic nesmí blokovat zobrazení;
číslování se nezapočítává do kopírovaného textu. PDF při přechodu na jinou stránku
zachová přesnou ruční úroveň zoomu, zatímco Fit přepočítá podle rozměrů nové stránky.
Každé nové PDF se otevře v režimu Fit window, který malé stránky i zvětší.
Typografické ovládání Markdownu je v jednom responzivním řádku.
[Ověření ovládání](../../verification/2026-11-08-viewer-controls.md) odděluje
testy skutečných oken od restartových kontrol. Následné
[měření latence](../../verification/2026-11-08-viewer-latency.md) prokázalo skrytý
softwarový fallback; Windows nyní vybírá skutečné WGPU/DX12 a debug sestavení
optimalizuje i kritické smyčky softwarového rendereru.

Nativní Browse/Stat/Storage a čtení pro F3 nyní používají samostatné Windows/Linux
filesystem knihovny přes [veřejné API](../../developer/NATIVE_FILESYSTEM_API.md).
Chybějící plugin nesmí aktivovat skrytý vestavěný fallback. Převod ostatních operací,
watcherů a obecných identit providerů zůstává samostatnou částí migrace.
Rekurzivní obyčejný Move odstraňuje každý soubor ihned po ověření a prázdné zdrojové
složky až po jejich obsahu; Skip ponechá příslušné předky. Platí explicitní
[platformní a metadatové hranice](../../verification/2026-11-08-recursive-move.md).

[Nový prázdný soubor](2026-11-05-empty-file-design.md): tlačítko v panelu a Shift+F4,
dialog názvu, výhradní vytvoření na backendu bez přepsání existujícího souboru.

Viewer etapa: implementované [veřejné API/SDK](../../../api/README.md), bezpečnostní
kontroly loaderu, sdílený Unicode renderer a samostatné pluginy text/hex, source
a Markdown. F3 vybírá plugin podle přípony, Alt+F3 žádá základní textový režim.
Pluginy zůstávají v jednom Git repozitáři, každý ve vlastním projektu/testech.
Aktuální hranice a testy popisuje [ověření viewerů](../../verification/viewer-implementation.md)
a [developer dokumentace API](../../developer/README.md). Filesystem pluginy
a zbývající migrace nativních operací pokračují podle aktuálního backlogu.

Aktuální drobná rozšíření: [unikátní číslované Hot Paths](2026-11-06-unique-numbered-hot-paths.md)
se správou zkratek bez zavření dialogu; [souhrn potvrzení mazání](2026-11-06-delete-summary.md)
s počty a součtem velikostí místo seznamu všech souborů. Light a Dark nyní stejně
jako ostatní schémata barevně odlišují složky; pravidla kontrastu jsou v
[návrhu témat](2026-11-06-theme-expansion-design.md). Výběr knihoven pro budoucí
obrázkový viewer a mediální náhledy upřesňuje
[návrh thumbnailů](2026-11-05-panel-views-thumbnails-design.md).

Další implementované úpravy: [F7 pro absolutní i relativní cesty](2026-11-06-create-any-path.md),
[aktivace panelu kliknutím](2026-11-06-panel-activation.md),
[přehlednější atributy a časy](2026-11-06-metadata-dialog-refinement.md)
a [sjednocené zobrazení chyb](2026-11-06-error-presentation-design.md).

Dialog atributů používá [jednosloupcové rozložení](2026-11-08-compact-attributes.md)
a jedno společné „Include subfolders“ nad spodními tlačítky, mimo posuvný obsah
záložek. Volba platí pro atributy, časy i změnu velikosti písmen názvu současně;
přepnutí záložky nemění její hodnotu ani umístění.

[Kontrolní součty na Ctrl+F8](2026-11-06-checksums-design.md) mají backend a
samostatné nemodální okno: MD5/SHA-1 pro kompatibilitu, výchozí SHA-256 a volitelné
SHA-512, zachycený výběr, neblokující průběh, kopírování hashů a bezpečný export.
Přesný stav ověření uvádí [záznam testů](../../verification/2026-11-07-checksum-window.md).
Ctrl+F7 zůstává změně velikosti písmen názvu. Ověřování kopie nadále používá SHA-256.

[Dialog kopírování F5](2026-11-07-copy-dialog-design.md) zachytí výběr a cestu
druhého panelu, umožní upravit cílovou složku a zařadí úlohu přímo do společné
ověřované fronty. Nepoužívá systémovou schránku. Výslovné Merge/Merge all a
obnovitelné Replace/Replace all doplňují nativní transferové schopnosti;
nahrazený originál zůstává v recovery složce. F6 a Cut/Paste sdílejí potvrzovací
dialog a ověřovanou frontu. Přesun maže každý zdrojový soubor bezprostředně po
jeho vlastním úspěšném zápisu, nezávislém SHA-256/readback ověření a nativních
kontrolách; další soubor se začne kopírovat až potom. První mazací schopnost je
omezena na běžné soubory na lokálním NTFS ve Windows. Linux přesun nyní odmítá
před publikováním, protože vyloučení rozpracovaného otevření zdroje není zaručeno.
Rozsah, limity a zbývající ověření určuje [transferový kontrakt](../../developer/TRANSFERS.md).

## 1. Záměr a potvrzená rozhodnutí

Vznikne svižný desktopový souborový manažer inspirovaný Altap Salamanderem a Total Commanderem. Začne malým, použitelným základem a bude se rozšiřovat po samostatně ověřitelných etapách.

Požadavky uživatele:

- Rust, UI ve Slintu, Tokio pro asynchronní orchestrace.
- Všechny knihovny třetích stran musejí umožnit zamýšlený vývoj, použití a distribuci
  bez povinných poplatků či royalties; preferujeme open source. Platí i pro
  transitivní závislosti, API/SDK, pluginy, build a testy. Licence a povinnosti
  eviduje [licenční politika](../../DEPENDENCY_POLICY.md) a
  [úplný seznam knihoven](../../THIRD_PARTY.md).
- Oddělený front-end a back-end. Uživatel zvolil **jednu aplikaci s oddělenými Rust crates a komunikací přes zprávy**.
- Dva nezávislé panely, zobrazení disků/připojených svazků a přepínání mezi nimi.
- Kopírování s povinným ověřením kontrolním součtem.
- Pomalé operace nesmějí blokovat ovládání aplikace.
- Windows x64 a Linux, zejména Ubuntu a Mint; macOS později.
- GPU využít tam, kde má měřitelný přínos.
- Možnost později využít C++ zdroje Salamanderu jako podklad pro chování a integraci s Windows.
- Dodatečně potvrzeno: bezpečnost má nejvyšší prioritu; všechny operace mimo prezentaci a vstup běží v back-endu, včetně úklidu velkých dat.
- Dodatečně potvrzeno: komplexní testy a **plná podpora Unicode v celém produktu**; to je zásadní rozdíl oproti referenci Salamanderu. Žádné ANSI převody názvů/cest. Textové Windows API musí používat varianty `W`; Unicode se svévolně nenormalizuje a zobrazení není identita souboru.
- Dodatečně potvrzeno: **žádný aplikační maximální počet znaků/bajtů cesty**, zejména žádné omezení na Windows `MAX_PATH`. Cesty mají dynamickou délku. Budoucí zdroje zahrnují také FTP a další vzdálené providery; jejich cesty nejsou lokální Windows cesty.
- Dodatečně potvrzeno: Windows, Linux a další platformní implementace jsou v oddělených souborech/crates a implementují společná rozhraní. Nepřidávat těla implementací pro několik OS do jednoho souboru.
- Dodatečně potvrzeno: veřejný Rust Style Guide a Rust API Guidelines, `rustfmt` s cílem 100 znaků na řádek, pevný limit ručně psaného zdrojového souboru **100 KB = 100 000 bajtů**. Podrobnosti a výjimky pro generovaný kód jsou v `CODE_STYLE.md`.
- Dodatečně potvrzeno: multiselekce, dávkové kopírování vybraného seznamu s průběhem a společná fronta kopírování/přesunu/mazání. Další operaci lze neblokujícím způsobem zadat během běhu jiné úlohy; zpracování nesmí zmrazit UI.
- Dodatečně potvrzeno: úplná spodní lišta F1–F10, příkazový editor se spouštěním výslovně zadaných příkazů v systémovém terminálu, Ctrl+[ / Ctrl+] vloží cestu levého/pravého panelu, Alt+Enter zobrazí vlastnosti, řádek `..`, systémové ikony a samostatná lišta disků/speciálních umístění nad každým panelem. Windows podporuje Shift+písmeno disku pouze při zaměření seznamu souborů.
- Dodatečně potvrzeno: Ctrl+Shift+S počítá na pozadí vybrané složky aktivního panelu, nebo všechny složky, když výběr neexistuje; druhý stisk výpočet ruší. Aktivní panel je vizuálně zvýrazněný. Průběžné/neúplné velikosti se odlišují od dokončených výsledků. Odkazy se při výpočtu nepřecházejí; výsledek je živý součet logických délek, nikoli atomický snapshot.
- Aktuální etapu rozšiřuje [návrh operací a ovládání](2026-11-05-rename-delete-scroll-design.md): rename/delete/F7, fronta, F11/F12, editace cesty, trvalé karty, locale a režimy panelů/náhledy. Má přednost před staršími read-only omezeními níže.
- Dodatečně potvrzeno: oba viditelné panely sledují změny souborového systému přes samostatné platformní rozhraní. Události se sloučí: obnova po 1 sekundě klidu, nejpozději po 3 sekundách souvislého proudu změn. Periodická kontrola po 15 sekundách zachytí chybějící notifikace, zejména síťových disků. Registrace watcherů, porovnání i obnova dat běží na backendu; výběr, kurzor, scroll a rozepsaný příkaz se zachovají.
- Dodatečně potvrzeno: odstranit horní řádek s názvem aplikace/About, aby neubíral místo panelům. Přidat uživatelské nastavení s výběrem původního modrého, neutrálního tmavého a světlého tématu. Nabídnout kompaktní rozestupy pro desktop/myš a větší rozestupy a dotykové cíle pro touchscreen. Odkaz About zůstane v menu; Options bude u příkazového řádku a v menu.
- Dodatečně potvrzeno: velká složka nemá zdržovat výsledky menších složek. Povolit omezené dávky souběžného počítání na prokazatelně lokálním SSD; na HDD, síťových discích a neznámém/nejistém úložišti pouze jednu aktivní I/O úlohu. Souběh musí být omezený a rozpočet sdílený mezi panely. Síťové úložiště se nepovažuje za SSD podle rychlého připojení nebo cache.
- Dodatečně potvrzeno: vestavěný viewer má mít prostý text, zvýrazněný zdrojový kód a vykreslený Markdown. Nejprve navrhnout rozhraní a zkontrolovat viewer/pluginy Salamanderu; F3 se aktivuje až se skutečnou implementací.
- Dodatečně odsouhlaseno: opakované sudo má využít systémové ověření a vhodnou otevřenou relaci, nikoli ukládat heslo v aplikaci. Automatické znovupoužití z editoru vyžaduje samostatný bezpečný návrh relací z oddílu 24; samotné otevřené root okno tento routing nezajišťuje.
- Dodatečně potvrzeno: obecné VFS rozhraní má umožnit lokální disky, procházení archivů včetně vnořených, další vzdálené filesystémy a Windows Registry. Současné lokální panely se převedou na první nativní provider; Windows/Linux/macOS mají každý svou samostatnou nativní implementaci.
- Dodatečně potvrzeno: každý viewer a filesystem včetně nativního je dynamicky načitatelný plugin vedle executable ve složkách viewers/filesystems. Přítomnost balíčku určuje dostupnost funkce. První viewer je text se syntax highlighting; první archivy ZIP, GZIP, TAR.GZ a 7z. Rozhraní a přejímku finalizuje [plugin návrh](2026-11-05-dynamic-viewer-filesystem-plugins-design.md).
- Dodatečně potvrzeno: při kopírování/přesunu na existující cíl zobrazit varování a rozhodnutí o přepsání se srovnáním zdroje/cíle, zejména velikosti a data/času. Bez výslovné platné politiky se nic nepřepíše.
- Dodatečně potvrzeno: výběr shellu vlevo od příkazového editoru na všech platformách: PowerShell/cmd na Windows, Terminal/Terminal (sudo) na Linuxu a stejné připravené rozhraní pro budoucí macOS. Enter při zaměřeném editoru spouští přesně jeho obsah ve zvolené externí relaci; volba se zachytí společně s cwd a oprávněním.

Ostatní konkrétní volby v tomto dokumentu jsou doporučení autora návrhu, nikoli další potvrzené požadavky uživatele. Zejména jde o BLAKE3, pořadí etap, počáteční rozsah metadat a navržené výkonnostní cíle.

## 2. Zvažované přístupy

| Přístup | Výhody | Náklady | Rozhodnutí |
| --- | --- | --- | --- |
| Jeden proces, oddělené crates, zprávy | Nízká režie, jednoduché balení, nezávisle testovatelný back-end | Pád procesu zasáhne celou aplikaci; blokující systémové volání nelze obecně násilně ukončit | Zvoleno uživatelem |
| Samostatný proces pro UI a back-end | Izolace pádů, možnost ukončit pracovní proces | IPC, přenos dat, verzování protokolu, složitější životní cyklus | Odloženo |

Vybraná architektura umožní později přesunout konkrétní rizikovou integraci do pomocného procesu. Nevyžaduje kvůli tomu dnes serializovat veškerou interní komunikaci ani zavádět síťové API.

## 3. Co přesně znamená neblokující aplikace

UI vlákno zpracovává vstup, kreslení a krátké aktualizace modelu. Neprovádí diskové operace, dotazy na síťové cesty, získávání ikon z OS, hashování ani řazení velkých seznamů. Ani uvolňování rozsáhlých snapshotů/indexů nesmí skrytě běžet v callbacku UI; taková data se předají omezené back-endové frontě k uvolnění. Nečeká na back-end pomocí `block_on`, `blocking_send`, `join` nebo dlouho drženého zámku.

U běžného souborového I/O Tokio používá blokující operace na pracovních vláknech. `async fn` tedy samo o sobě nezaručuje neblokující systémové I/O. Návrh zaručuje oddělení této práce od UI a omezení spotřeby prostředků; neslibuje okamžité dokončení ani okamžité zastavení libovolného volání OS. [Tokio: filesystem](https://docs.rs/tokio/latest/tokio/fs/index.html)

Práce bude rozdělena následovně:

| Oblast | Umístění | Pravidlo |
| --- | --- | --- |
| UI a Slint modely | Hlavní vlákno | Pouze krátká práce s daty v paměti |
| Fronty, plánování, časovače, koordinace úloh | Vícevláknový Tokio runtime | Bez dlouhých CPU výpočtů a blokujících volání v běžných async tasks |
| Blokující OS a souborové operace | Omezené pracovní kapacity | Omezení souběhu před spuštěním práce, sledování i po požadavku na zrušení |
| Velké řazení, hashování, budoucí dekódování náhledů | Omezená CPU/pracovní kapacita | Kopírování a hashování lze zpočátku sloučit do jednoho streamujícího workeru |

Krátké výpočty typu změny kurzoru zůstanou synchronní. Veškeré potenciálně pomalé operace mají asynchronní rozhraní vůči UI.

## 4. Rozdělení projektu

Cargo workspace rozděluje pět logických vrstev do následujících crates; platformní vrstva má oddělené implementace:

| Crate | Odpovědnost | Závislosti uvnitř workspace |
| --- | --- | --- |
| `midnight-core` | Doménové typy, identifikátory, příkazy/události, chyby, rozhraní providerů a jejich schopnosti | Žádné |
| `midnight-engine` | Navigace, načítání adresářů, plánování, kopírování, ověřování a stav úloh; Tokio | `midnight-core` |
| `midnight-platform` | Malá továrna/reexport podle targetu | `midnight-core` a zvolený platformní crate |
| `midnight-platform-common` | Přenositelné bezpečné Rust operace a společné pomocné struktury | `midnight-core` |
| `midnight-platform-windows` | Windows implementace služeb, Win32, svazky a Windows specifika | `midnight-core`, `midnight-platform-common` |
| `midnight-platform-linux` | Linux implementace služeb, mountinfo a Linux specifika | `midnight-core`, `midnight-platform-common` |
| `midnight-ui` | `.slint` soubory, klávesové ovládání, presenter, modely a převod dat pro zobrazení | `midnight-core` |
| `midnight-explorer` | Spuštění, konfigurace, vytvoření runtime, propojení zpráv, injektování providerů a ukončení | Ostatní crates |

`midnight-core`, `midnight-engine` a `midnight-platform` neznají Slint. `midnight-ui` nevolá filesystem ani konkrétní engine a není závislé na jeho implementaci. `midnight-explorer` dodá presenteru neblokující vstupní rozhraní a propojí výstupní události s UI. Rozhraní providerů budou odpovídat potřebám aktuálních etap; nevznikne předem rozsáhlá abstrakce všech souborových systémů.

Pro lokální cesty se používá dynamický `PathBuf`/`OsString`, nikoli cesta rekonstruovaná z textu v UI nebo buffer o velikosti `MAX_PATH`. `PathBuf` je lokální reprezentace, nikoli univerzální formát pro FTP či jiné providery. Zobrazený název a identita položky jsou oddělené. Rust cesty nemusejí být platné UTF-8. [Rust: Path](https://doc.rust-lang.org/rust-by-example/std_misc/path.html)

Základní identifikátory: `PaneId`, `NavigationId`, `EntryId`, `JobId`, `VolumeId`. `EntryId` identifikuje položku v konkrétním výpisu; OS identita souboru je samostatná informace. Výběr ani příkazy se neodvozují jen z indexu řádku, který se může změnit řazením.

## 5. UI a komunikace s back-endem

Slint event loop i komponenty žijí na hlavním vlákně. Samostatně vytvořený Tokio runtime obsluhuje back-end. Výsledky se doručují přes bridge; změny Slint modelů se aplikují výhradně v UI vlákně. [Slint: threading](https://docs.slint.dev/latest/docs/rust/slint/#threading-and-event-loop)

Počáteční smlouva obsahuje zejména:

- Příkazy: `ListVolumes`, `Navigate`, `Refresh`, `StartCopy`, `CancelJob`, `ResolveConflict`.
- Události: `VolumesUpdated`, `ListingStarted`, `EntriesBatch`, `ListingFinished`, `ListingFailed`, `JobStateChanged`, `ConflictRaised`.
- Poslední průběžný stav: počet zpracovaných bajtů, aktuální soubor a fáze úlohy.

Jde o návrh významu zpráv, nikoli hotové Rust API. Přesné signatury a reprezentace async rozhraní jsou implementační volba.

Komunikace má tři režimy:

1. **Příkazy a rozhodnutí:** omezená fronta; callback UI nikdy nečeká na její uvolnění. Při zaplnění se příkaz nepovažuje za přijatý a UI zobrazí odpovídající stav. Požadavek na zrušení má samostatnou neblokující signalizaci.
2. **Výpisy adresářů:** omezené dávky a omezená fronta. Producent čeká v back-endu, pokud UI nestíhá. Každá dávka obsahuje panel a generaci navigace.
3. **Průběh úloh:** uchovává se poslední hodnota; není nutné doručovat každý mezikrok. Změny terminálních stavů a požadavky na rozhodnutí jsou spolehlivé, bez tichého zahazování.

Engine vlastní autoritativní registr úloh. Zavření zobrazení průběhu úlohu neztratí. UI může načíst aktuální stav znovu.

Omezení front platí až k UI: bridge nesmí vyprázdnit omezený kanál do neomezeného množství `invoke_from_event_loop` callbacků. Drží nejvýše jedno čekající probuzení; UI spotřebuje práci v časovém rozpočtu a případně naplánuje další průchod. Počty zpráv a běžné dávky položek jsou omezené; velké snapshoty se předávají sdíleným neměnným vlastnictvím. Rozpočet paměti nebo transportní rámec nesmí zavést skrytý maximální počet znaků cesty: dlouhá identita se přenáší sdíleně/tokenem či streamem a nedostatek prostředků vrací explicitní chybu bez zkrácení. [Tokio: mpsc](https://docs.rs/tokio/latest/tokio/sync/mpsc/), [Slint: invoke_from_event_loop](https://docs.slint.dev/latest/docs/rust/slint/fn.invoke_from_event_loop)

Při rychlém přepnutí A → B → C se starší výpisy zruší kooperativně a jejich opožděné dávky se podle `NavigationId` ignorují. Stejnou generaci nesdílí oba panely.

## 6. První podoba dvou panelů

Oba panely mají stejnou komponentu a vlastní stav: umístění, historii, řazení, kurzor, výběr, generaci navigace a stav načítání/chyby.

Každý panel obsahuje:

- Výběr disku/připojeného svazku a editovatelnou cestu.
- Seznam se sloupci název, velikost a datum změny; adresáře jsou standardně první.
- Odlišený kurzor, výběr a aktivní panel; tyto stavy se neslévají.
- Stavový řádek s počtem položek a vybraných souborů; volné místo se načítá nezávisle.

Základní ovládání: Tab mezi panely, Enter nebo levý dvojklik otevře adresář v panelu či soubor ve výchozí systémové aplikaci, Backspace nadřazený adresář, šipky a Page Up/Down pohyb, Space/Insert výběr, F5 kopírování do druhého panelu po jeho zavedení. Levé kliknutí nastaví kurzor i aktivní panel; Ctrl+klik přepíná multivýběr. Zkratky budou definované jako akce, aby šly později přemapovat. Původní rozložení klávesových zkratek pro konkrétní OS ověříme v UI etapě.

Myší se aktivuje pouze levý dvojklik bez modifikátorů. Výběr, pravé kliknutí,
hover ani automatický refresh soubory nespouštějí. Token řádku zachycuje aktuální
navigační záměr, generaci skutečně zobrazeného výpisu a identitu položky; změna
řazení nebo výpisu mezi stisky nesmí otevřít náhradní položku. Aktualizace kurzoru
mění jen dotčené řádky, protože reset modelu by zrušil Slint detekci dvojkliku.

Nové `OpenService` klasifikuje explicitně otevřený odkaz na backendu: adresář
naviguje uvnitř panelu, běžný soubor použije systémovou asociaci, speciální nebo
chybějící soubor vrátí chybu s původní cestou. Windows má samostatnou implementaci
`ShellExecuteExW(open)` s dynamickými UTF-16 cestami a oddělenými parametry; Linux
použije absolutní systémový `xdg-open` nebo `gio open` s doslovným nativním
argumentem bez shellu. Metadata, spuštění i sklizeň pomocných procesů běží mimo
UI, s vlastním limitem čtyř volání a šestnácti sledovaných procesů. Starý výsledek
odkazu nesmí přepsat novější navigaci ani ukrást zaměření druhému panelu. Nejde
o garanci neměnné inode identity proti externím souběžným změnám souborového
systému; budoucí zápisy potřebují silnější smlouvu. Podrobnosti a referenční
kotvy Salamanderu jsou v `2026-11-04-mouse-activation-design.md`.

Aktuální rozšíření ovládání je podrobně sjednocené v oddílech 18–21: celé F1–F10,
příkazový editor, vlastnosti, lišty umístění, počítání velikostí, řazení, watchery,
témata a hustota. Horní branding řádek se nezobrazuje. Aktivní panel odlišuje
barevné záhlaví a jemný rámeček; kurzor a multivýběr zůstávají samostatné stavy.

Seznam bude virtualizovaný: pro viditelné řádky se vytvářejí UI prvky, nikoli samostatná komponenta pro každý soubor. Návrh počítá s `ListView`, vlastními řádky a společnou hlavičkou sloupců. Model vrací pouze data již v paměti, nikdy neprovádí I/O. Virtualizace sama neomezuje paměť potřebnou pro uložené položky ani cenu řazení. [Slint: ListView](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/views/listview/)

Jména se zobrazují postupně v dávkách. Dražší metadata se doplňují odděleně; neznámá velikost se nezobrazuje jako nula. Řazení velkých výpisů probíhá mimo UI a výsledky se aplikují s kontrolou generace. Aktualizace průběhu kopírování neobnovuje model celého adresáře. Velikost adresářů se při pouhém otevření panelu rekurzivně nepočítá.

## 7. Disky a rozdíly platforem

Společný `VolumeService` vrací seznam navigačních kořenů a jejich stav. UI nebude předpokládat, že svazek má písmeno disku.

- **Windows:** písmena disků a kořeny svazků přes Win32 adaptéry; názvy, volné místo a dostupnost se zjišťují odděleně. Později historie UNC cest a další integrace. [Microsoft: GetLogicalDriveStringsW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getlogicaldrivestringsw)
- **Linux:** kořen `/`, domovský adresář jako navigační zkratka a vhodně filtrované připojené filesystemy. Zdroj informací o mount namespace je `/proc/self/mountinfo`; interní pseudofilesystemy se standardně nezobrazují. V první etapě se přepíná mezi již připojenými svazky; připojování a bezpečné odpojování zařízení je samostatná pozdější funkce. [Linux kernel: mountinfo](https://www.kernel.org/doc/html/latest/filesystems/proc.html)
- **macOS později:** nový adaptér pro svazky, přístupová oprávnění a distribuci; doménový engine zůstává společný.

Nedostupný svazek má vlastní stav chyby. Na dotaz na jeho kapacitu se nečeká před vykreslením ostatních svazků. Vyčerpání kapacity kopírovacích workerů nesmí zablokovat veškerou kapacitu pro navigaci. Trvale zablokované systémové volání však může držet worker; návrh omezuje počet takových pokusů a nepřidává neomezeně další vlákna.

Nativní notifikace změn budou optimalizace. Při chybě, ztrátě událostí či návratu do panelu se použije nový výpis. Watcher není jediný zdroj pravdy, zejména na síťových filesystémech. [notify: known problems](https://docs.rs/notify/latest/notify/#known-problems)

## 8. Kopírování s povinným ověřením

### Algoritmus a rozsah záruky

Implementované povinné ověřování používá **SHA-256**, průběžně počítaný nad bloky
dat ze zdrojového handle a porovnaný s úplným nezávislým čtecím průchodem cíle.
Kopírování i přesun zachovávají jediný aktivní buffer o velikosti 256 KiB;
změna algoritmu by vyžadovala samostatně ověřený kontrakt.

Úspěšné kopírování běžného souboru znamená:

> Bajtový proud přečtený ze zdrojového handle a samostatný úplný čtecí průchod cílovým souborem mají shodnou délku a SHA-256 hash. Nebyla zjištěna změna zdroje. Proběhlo požadované dokončení zápisu, synchronizace a publikování cíle.

Hash ověřuje běžný datový obsah. Zachování práv, ACL, NTFS alternate data streams, rozšířených atributů, hardlinků a sparse rozložení představuje samostatné schopnosti a není součástí prvního slibu bezeztrátové kopie. Kopírovací UI musí předem uvádět dostupnou politiku metadat; selhání slíbeného zachování metadat se nesmí vydávat za úplný úspěch.

### Implementovaný první řez COPY-03 (2026-11-07)

Existující API zůstávají `ContentOnly`. Výslovné `RequireModifiedTime` podporuje
nejvýše 64 různých nativních běžných souborů na Windows/Linuxu; složky, odkazy a
virtuální zdroje odmítá bez tichého snížení záruky. UI volba zachování přesného
času změny je ve výchozím stavu vypnutá; její testy UI prošly na Windows i Linuxu
a aktuální Windows release prošel nativním ověřením spuštění.
[Ověření](../../verification/2026-11-07-metadata-filters-provider-checkpoint.md),
[kontrakt](../../developer/TRANSFERS.md) a
[volba dialogu](../../developer/TRANSFER_OPTIONS.md) vymezují skutečný rozsah.

Backend zachytí čas ze zdrojového handle před čtením dat. Po zdrojovém SHA-256 a
úplném nezávislém čtení cíle nastaví čas na vlastním neveřejném souboru, provede
synchronizaci a přesný metadata readback. Typy a schopnosti celé dávky se ověřují
před zápisem dat; konkrétní nepodporované rozlišení cíle se může zjistit až po
ověření dat, před publikováním daného souboru. Dřívější dokončené cíle zůstanou;
chyba daného souboru zabrání jeho smazání a zastaví další přesun. Časové kontroly pouze
doplňují úplné hashe a nové nativní revize; publikovaný cíl se nikdy neopravuje,
aby se skryla změna. Dřívější dokončené přesuny zůstávají dokončené; při chybě
zůstane dosud nesmazaný zdroj zachován. Požadovaný běžný přesun nepoužívá
zdrojové recovery složky ani volbu „Move with recovery“; starší implementace se
nahrazuje podle nejnovějšího dodatku výše. Otevřené čtecí handly mohou
odložit fyzické uvolnění místa; přesný počet uvolněných bajtů se neslibuje.

Ostatní časy, atributy, práva, ACL, xattrs, proudy, metadata adresářů a zachování
odkazů tím nejsou dokončeny. Obecný přesun adresářů zůstává samostatnou etapou.

### Postup pro jeden běžný soubor

1. Otevřít zdroj jednou a zaznamenat identitu a dostupné indikátory změny. Odmítnout kopii do téhož souboru, včetně detekovatelných aliasů.
2. Výhradně vytvořit vlastní dočasný soubor vedle cíle na stejném filesystemu. Název není předvídatelný a nesmí přepsat cizí soubor. Práce s handle a ověřování identity omezí závody při opakovaném vyhodnocování cest.
3. Po omezených blocích číst zdroj, počítat jeho hash a zapisovat všechna přečtená data. Nepoužívat načtení celého souboru do RAM. Velikost bloků bude měřená implementační volba.
4. Dokončit čekající zápisy, aplikovat slíbená metadata a provést synchronizaci souboru. `flush()` Tokia a synchronizace úložiště jsou odlišné kroky; chyba kteréhokoli povinného kroku je chyba úlohy.
5. Novým úplným čtecím průchodem ověřit dočasný cíl: vlastní handle se převine, případně se bezpečně znovu otevře se shodnou identitou. Porovnat délku a hash. Opětovné zahashování odesílaného bufferu není ověření cíle.
6. Zkontrolovat dostupné indikátory změny zdroje. Při zjištěné změně soubor neoznačit jako úspěšně zkopírovaný.
7. Publikovat ověřený soubor pod finálním názvem podle politiky kolize, pomocí platformního adaptéru. Původní cíl se předem nemaže.
8. Dokončit podporované kroky synchronizace adresářového záznamu. Až potom oznámit slíbený úspěch. Pokud se publikování povedlo, ale následná synchronizace selhala, výsledek výslovně uvádí „cíl publikován, trvalost nepotvrzena“.

Základní politika při kolizi je **přeskočit nebo zvolit jiný název**. Publikování bez přepsání musí řešit souběžný vznik cíle atomicky, nikoli kontrolou existence před běžným rename. Ověřené přepisování existujícího cíle se přidá po otestování platformních pravidel. Při nepodporované záruce se operace odmítne s vysvětlením; nepřejde tiše na smazání původního cíle.

Stejný filesystem umožní používat vhodné rename/publish operace; jejich podmínky a chování při selhání nejsou mezi Windows a Linuxem totožné. Platformní adaptér deklaruje podporované schopnosti. Referenční testování začíná lokálními NTFS a ext4, další filesystemy a síťové servery rozšíří ověřenou matici. Nelze obecně tvrdit, že libovolné přepsání je bezpečná transakce. [Rust: rename](https://doc.rust-lang.org/std/fs/fn.rename.html), [Microsoft: ReplaceFileW a jeho chybové stavy](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)

### Omezení, která mají význam pro správnost

- Nové přečtení cíle může být obsloužené cache OS. Ověření obsahu a odolnost při výpadku napájení jsou odlišné vlastnosti. Ani hash se synchronizací nepředstavuje absolutní záruku proti všem poruchám hardwaru. [Microsoft: file caching](https://learn.microsoft.com/en-us/windows/win32/fileio/file-caching)
- Kontrola velikosti a času změny zachytí běžné změny zdroje, ale neprokazuje konzistentní snapshot během souběžného zápisu. Silnější režim později potřebuje vyloučení zapisovačů nebo snapshot daného filesystemu.
- Základní ověřování znamená jeden čtecí průchod zdrojem, zápis cíle a další čtecí průchod cílem. Jeho cenu měříme jako součást celkové rychlosti. Odezva UI zůstává nezávislá na době operace.
- Uživatelský panel s průběhem rozlišuje „kopírování“, „synchronizace“ a „ověřování“. Zapsání posledního bajtu neznamená hotovo.

### Omezený souběh podle skutečného úložiště

Pro budoucí kopírování použít nejprve souběh **různých souborů**, každý s vlastním
dočasným cílem, sekvenčním datovým proudem a samostatným úplným ověřením. Návrhový
výchozí limit je **2 aktivní I/O proudy na prokazatelně lokálním SSD**; nastavitelný
strop 4 se povolí pouze po měření celého přenosu včetně synchronizace a readbacku.
HDD, síť a neznámé úložiště začínají jedním proudem ve sdíleném rozpočtu příslušného
zařízení či konzervativní skupiny. SSD na vzdáleném serveru není lokální SSD.
Jde o konzervativní návrhové hodnoty, nikoli doporučení výrobců nebo garanci zrychlení.
Microsoft podporuje paralelní kopie v Robocopy; upozorňuje také, že příliš mnoho
vláken může výkon zhoršit. [Robocopy](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/robocopy),
[Microsoft: malé soubory a souběh](https://learn.microsoft.com/en-us/troubleshoot/windows-server/networking/slow-smb-file-transfer#slow-transfer-when-using-small-files)
Také desktopová měření výrobce SSD rozlišují queue depth a skutečnou odezvu;
maximální benchmarková propustnost neurčuje vhodný souběh této aplikace.
[Solidigm: PC Storage Performance in the Real World](https://www.solidigmtechnology.com/content/dam/solidigm/en/site/products/technology/performance-with-impact/documents/Performance-With-Impact-White-Paper-R2.pdf)

Počet vláken není počet I/O proudů ani hardwarová queue depth. Tokio koordinuje
omezené úlohy na backendu; skutečně blokující I/O běží v omezené pracovní kapacitě.
Zdrojové čtení, cílový zápis, synchronizace a ověřovací čtení sdílejí rozpočet
skutečného fyzického úložiště napříč panely a operacemi. Aliasy a oddíly C:/D: na
témže SSD nedostanou dva nezávislé rozpočty. Každé probíhající I/O se účtuje;
sekvenční fáze jednoho souboru na stejném zařízení mohou využít jeden permit,
souběžné čtení/zápis by vyžadovalo další. Neznámá topologie volí sdílený
konzervativní režim. Permit zůstává obsazen do skutečného návratu OS volání i po
zrušení. Přednost navigace, omezené fronty, bajty bufferů a CPU hashování jsou
samostatné podmínky přijetí práce, nikoli jen limit počtu tasks.
Prokázaná různá fyzická zařízení mohou překrývat fáze různých souborů. Velké
soubory po omezených úsecích a skutečném dokončení I/O uvolní pracovní permit
další připravené práci; jejich handle/offset/hash přitom zůstávají vlastněné a
omezené. Před zapnutím kopírování musí stejný rozpočet zařízení zahrnovat také
skeny velikostí a interaktivní čtení, aby se nezávislé limity nesčítaly.

Paralelní bloky jednoho velkého souboru jsou odložená, samostatně měřená
optimalizace. Zachovají přesné pořadí bajtů BLAKE3, omezené přeuspořádání a původní
publikační kontrakt; hashování bloků v pořadí dokončení nebo slepení jejich hashů
není hash původního souboru. Stávající čtyřslotový vážený scheduler velikostí
složek není hotový kopírovací algoritmus. Podrobnosti rozpočtů, férovosti,
publikování a testů určuje [návrh souběhu kopírování](2026-11-04-copy-concurrency-design.md).
**Toto je pouze návrh; kopírování/přesun/mazání zůstávají neimplementované a vypnuté.**

## 9. Stav úloh, rušení a chyby

Základní fáze: `Queued → Preparing → Copying → Syncing → Verifying → Publishing → Completed`. Vedlejší stavy: `WaitingForDecision`, `CancelRequested`, `Cancelled`, `Failed` a výsledek po publikování s nepotvrzenou trvalostí.

Engine vlastní všechny pracovní handles a sleduje i práci, jejíž UI už požádalo o zrušení. Rušení je kooperativní mezi bloky a fázemi. Již spuštěné `spawn_blocking` nelze zastavit pouhým `abort()`. Dočasný soubor se uklízí až po dokončení všech probíhajících operací nad ním. [Tokio: spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

Před publikováním zrušení zabrání publikování a následně odstraní vlastní dočasný soubor. Pokud už publikování začalo, vyčká se na jeho skutečný výsledek; úspěšně publikovaný soubor se kvůli souběžnému stisku Zrušit nemaže. Stav v UI v mezidobí odpovídá „ruší se“, nikoli předčasnému „zrušeno“.

U adresářové úlohy už dokončené soubory zůstávají. Celá složka není jedna atomická transakce. Chyby jednotlivých souborů mají konkrétní výsledek a souhrn úlohy rozlišuje úplný úspěch, částečné dokončení a zrušení. Dialog rozhodnutí pozastaví danou úlohu, nikoli UI nebo vlákno čekající na odpověď.

Po pádu mohou zůstat dočasné soubory. Bezpečně se evidují jako artefakty konkrétní úlohy; neodstraňují se libovolné soubory podle přípony. Automatické pokračování v rozkopírovaném souboru není součást prvního kopírovacího milníku.

Fronta, paměť a souběh jsou omezené. Velké kopie nesmějí vyhladovět interaktivní
navigaci. První kopírovací milník má zahrnout konzervativní rozpočty podle
zdrojového/cílového úložiště z oddílu 8 a oddělené pracovní kapacity pro
interaktivní práci. Jeden task nebo nové vlákno pro každý z milionu souborů je
nepřípustný; čekající požadavky se připravují postupně v omezeném okně.

## 10. GPU a měření výkonu

GPU použijeme pro vykreslování Slintu. Počáteční kandidát je Winit + FemtoVG-WGPU; proti Skia se ověří startup, plynulost, paměť a velikost distribuce. Zůstane explicitně testovaná softwarová varianta pro prostředí bez použitelné GPU. Winit pokrývá Windows, Linux X11/Wayland a macOS; Slint nabízí více GPU i softwarových rendererů. [Slint: Winit](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit/), [Slint: renderery](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/)

Kopírování a hashování začnou na CPU. GPU hashování má přenosovou a synchronizační režii; jeho případné zavedení vyžaduje měřitelný přínos celému přenosu s ověřením. Vícevláknové hashování není automaticky zapnuté jen instalací BLAKE3; nesmí přesytit stroj ani zhoršit přístup k HDD.

Navržené cíle, které se musí potvrdit na zaznamenaném referenčním stroji:

- Při běžném ovládání cílit na 60 fps; aplikační aktualizace UI dávkovat přibližně do 2–4 ms na průchod.
- Při současném kopírování a výpisu 100 000 položek cílit na p95 odezvy vstupu do 50 ms; sledovat i p99 a nejdelší zásek.
- U lokálního zahřátého adresáře cílit na první použitelné položky do 100 ms. U studeného či nedostupného média se měří zvlášť a UI ihned zobrazuje načítání.
- Průběh kopírování obnovovat přibližně 10–20krát za sekundu; za nečinnosti zbytečně nepřekreslovat.
- Celková velikost kopírovaného souboru nesmí určovat velikost bufferu v RAM. Seznam adresáře může růst s počtem položek; paměť se měří odděleně od počtu viditelných řádků.

Výsledky vždy uvádějí OS, CPU, médium, filesystem, renderer, počet položek a stav cache. Měří se čas do prvních položek i úplného načtení, UI latency, paměť, zatížení CPU, obsazení front a celková rychlost kopie včetně synchronizace a ověření. Diagnostika začíná strukturovanými událostmi přes `tracing`, bez zaznamenávání obsahu souborů.

## 11. Etapy a jejich přejímací podmínky

### Etapa 1: skutečné dva panely a svazky

Vytvořit workspace, zprávový bridge a dva panely nad skutečným lokálním filesystemem. Zahrnout přepínání dostupných svazků, vstup cesty, historii, základní klávesové ovládání, postupné výpisy, řazení a stavy načítání/chyby. Ve výchozí podobě použít jednoduchý vzhled s jasným označením aktivního panelu.

Hotovo znamená: každý panel nezávisle naviguje, rychlé přepínání nevrací staré výsledky, pomalý dotaz na disk neblokuje UI a velký výpis vytváří pouze viditelné řádky. Windows a Linux mají průběžné sestavení a automatické kontroly od této etapy; testuje se alespoň jedna Ubuntu a jedna Mint instalace. Konkrétní minimální verze OS a distribuční formát se zvolí před balením, nikoli odhadem v doménovém jádru.

### Etapa 2: jeden nebo více běžných souborů s ověřením

Přidat F5, cílový panel, jednoduchou frontu, průběh fází, BLAKE3, dočasný cíl, úplný kontrolní čtecí průchod, synchronizaci, publikování bez přepsání a zrušení. Při kolizi přeskočit nebo vybrat jiné jméno. Zachovávat obsah a deklarovaná základní metadata; další typy položek zatím výslovně odmítnout.

Fronta od začátku používá společný model operací z oddílu 17. Dávka představuje
zachycený výběr více položek, nikoli jednu synchronní smyčku spuštěnou z UI.
Přesun/mazání se připojí ke stejné infrastruktuře po zavedení jejich bezpečných
platformních služeb; nebudou mít vlastní neslučitelné fronty.

Hotovo znamená: úspěch vzniká až po ověření; vložené poškození cíle je detekováno; existující cílový soubor se nezničí při chybě ani závodu; zrušení počká na doběhnutí workeru před úklidem. UI jde používat během celé operace.

### Etapa 3: adresáře, kolize a odolnost

Přidat postupný průchod stromem, multivýběr, rozhodování o konfliktech, ověřené přepisování na podporovaných filesystémech a souhrn jednotlivých výsledků. Výchozí politika pro symlinky/junctions je nenásledovat a uvést přeskočenou položku; zachování samotných odkazů se přidává jako explicitní schopnost. Běžné adresáře se kopírují bez čekání na úplný předběžný součet; do jeho zjištění je průběh neurčitý.

Zakázat kopii adresáře do sebe nebo vlastního podstromu, včetně zjistitelných aliasů; symlinky nesmějí způsobit smyčku. Existující watcher rozšířit o koordinaci s vlastními mutacemi a evidenci nedokončených artefaktů.

Hotovo znamená: otestované plné médium, odpojení disku, přístupová chyba, souběžná změna zdroje a cíle, zrušení v jednotlivých fázích a částečné dokončení stromu. Zachované zdrojové a cílové soubory odpovídají oznámenému výsledku; nepodporovaná metadata nejsou slibována.

### Etapa 4: další funkce po jedné

Na stejný model příkazů a úloh navázat přesun, přejmenování, vytváření složek, koš/mazání, hledání, záložky panelů, prohlížeč a náhledy. Přesun mezi filesystémy smí smazat zdroj až po úspěšné ověřené kopii a publikování. Přesun v rámci jednoho filesystemu je jiný typ operace s vlastními pravidly.

Archivy a vzdálené zdroje přidávat jako providery s deklarovanými schopnostmi: čtení, zápis, přejmenování, kontrolní čtení, publikování a politika metadat. Provider bez potřebné schopnosti nesmí předstírat stejnou záruku jako lokální ověřená kopie. Samostatné plugin API nebylo součástí původního základu; nový požadavek z 5. listopadu je zavádí jako podmínku viewer/VFS etapy, viz oddíly 22–23.

Počítá se výslovně s FTP/FTPS a případně SFTP; FTP, FTPS a SFTP jsou odlišné protokoly/providery. Jejich zavedení nesmí vyžadovat přepis UI ani kopírovacího stavového automatu. Detailní pravidla umístění a délky cest určuje následující oddíl 14.

macOS následuje po stabilizaci základních operací na Windows/Linuxu. Zachová se rozdělení UI, enginu a platformních adaptérů.

## 12. Ověřování návrhu při implementaci

- Engine lze spouštět bez Slintu nad testovacím providerem s řízenými chybami a latencí.
- Kritické testy kopie: poškozený cíl, krátké čtení/zápis, prázdný a velký soubor, selhání zápisu/flush/sync/publish, vznik cíle během kopírování a rušení v každé fázi.
- Testy zdroje: append, truncate, změna stejné velikosti a výměna položky během práce. Testy dokumentují limity detekce, nepředstírají snapshot.
- Integrační testy běží zvlášť pro platformní publish pravidla a Windows sharing violations. Názvy s Unicode, ne-UTF-8 na Linuxu, dlouhé cesty a filesystemová citlivost na velikost písmen jsou součástí matice.
- UI ověření zahrnuje souběžnou navigaci a kopírování, saturaci front, ignorování starých generací a práci s velkým adresářem.
- Po zavedení evidence nedokončených úloh testovat restart mezi dokončením zápisu, ověřením a publikováním. Samotné unit testy neprokazují odolnost vůči fyzickému výpadku napájení.

## 13. Zdroje Salamanderu a zbývající implementační volby

Dodané C++ zdroje v `D:\experiments\salamander-main` se průběžně kontrolují jako reference pro každou novou funkci, chování panelů a Windows hraniční případy. Implementace v Rustu vzniká nezávisle; kód, prostředky a řetězce Salamanderu se nekopírují. Konkrétní dohledané reference jsou v doplňujícím návrhu commander ovládání. Unicode a dynamické cesty jsou vlastní závazky produktu, nikoli převzaté limity Salamanderu.

Slint umožňuje desktopovou aplikaci pod více licenčními variantami. Konkrétní licenci projektu a případnou atribuci je potřeba určit před distribucí podle skutečně připnuté verze. [Slint: licence](https://github.com/slint-ui/slint/blob/master/LICENSE.md)

Implementátor může bez změny tohoto návrhu zvolit konkrétní datové struktury modelu, velikosti bloků a dávek, typ async provider rozhraní, počáteční limity workerů a testovací nástroje. Volby ovlivňující uživatelský slib — zejména změnu pravidel přepisování, vypnutí ověřování nebo sledování odkazů — vyžadují vědomou úpravu návrhu.

První implementační plán má podrobně rozpracovat pouze etapu 1. Další etapy zatím určují směr a hranice; jejich plán se zpřesní podle zkušenosti a měření z předchozí etapy. Podrobné průběžné bezpečnostní, integritní a Unicode invarianty jsou v `SECURITY.md`.

## 14. Cesty bez aplikačního limitu a vzdálené zdroje

### Délka a identita

- Aplikace nestanovuje žádnou univerzální maximální délku cesty, názvu ani URL. Nikde nevzniká `MAX_PATH`, `MAX_PATH + 1`, pole o 260 znacích nebo obdobný skrytý limit pro interní identitu, fronty, historii, konfiguraci, clipboard či příkazy operací.
- Ukládání používá dynamické struktury. Velikost se zjišťuje v jednotkách požadovaných konkrétním API: UTF-8 bajty, UTF-16 jednotky nebo providerem definované bajty; počet zobrazených znaků se za velikost bufferu nezaměňuje.
- Fyzicky nekonečná cesta není příslib: konkrétní filesystem, OS či server může mít vlastní omezení celkové cesty, jednotlivé komponenty, protokolu nebo dostupné paměti. Takové omezení pochází od providera a vrací se jako konkrétní chyba. Nikdy se neřeší tichým zkrácením, přepsáním jiné položky nebo převedením do ANSI.
- Zkrácení textu či horizontální posun ve vykresleném řádku je pouze vizuální. Operace, historie a kopírování textu vždy používají celou původní cestu. Ani dočasný cíl či ověřovací manifest nesmí používat zkrácenou identitu.
- Na Windows používat Unicode/long-path kompatibilní API a dle požadavku daného API rozšířené formy cest včetně UNC. Prefixy se nesmějí odstraňovat při skutečných operacích pouze kvůli vzhledu. Budoucí shell integrace musí buď zachovat tuto schopnost, nebo jasně oznámit omezení konkrétní externí operace. [Microsoft: dlouhé cesty](https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation)

### Umístění podle providera

Před zavedením prvního vzdáleného zdroje se lokální reprezentace v navigační smlouvě rozšíří na koncept:

`Location = ProviderId + ProviderPath`

`ProviderPath` uchovává bezeztrátovou, providerem vlastněnou identitu: pro lokální filesystem `PathBuf`, pro vzdálený server jeho cestu/token a pravidla kódování. Není to jeden univerzální `String`/URL předávaný lokálnímu `Path::join`. Zobrazený text je samostatná Unicode reprezentace.

Provider v back-endu provádí sestavení a rozlišení cesty, práci s kořenem/rodičem, kódování protokolu, pravidla velikosti písmen a interpretaci odkazů. UI posílá záměr navigace a nepronáší do vzdálených cest lokální oddělovače, písmena disků, `MAX_PATH` či normalizační pravidla Windows. Ověřování existence neprobíhá v UI.

Přihlašovací údaje jsou mimo identitu cesty a zobrazované URL; nepatří do historie, logů ani chybových hlášek. Síťový provider má omezený souběh, časové limity, kooperativní rušení a explicitní politiku opakování. Název souboru je parametr knihovny/protokolu, nikdy shellový příkaz nebo neověřený text vložený do řídicího řádku protokolu. Podpora starších serverových kódování neospravedlňuje tiché poškození Unicode; provider musí kódování vyjednat/zvolit a při nemožnosti bezeztrátového převodu vrátit chybu. [RFC 2640: internacionalizace FTP](https://www.rfc-editor.org/rfc/rfc2640)

### Ověřená kopie mezi providery

Stejný kopírovací engine zpracuje zdroj a cíl typu `Location`. Povinné ověření obsahu platí i při kopii na FTP nebo jiný vzdálený server: je nutný nezávislý úplný čtecí průchod cílem, případně později výslovně definovaný důvěryhodný ekvivalent s doloženou zárukou. Nepřítomnost serverového checksum příkazu není důvod ověřování vypnout; lze znovu číst obsah přes provider.

Schopnosti dočasného cíle, přejmenování/publikování, přepisování a trvalosti se vyhodnocují zvlášť. Vzdálený provider bez potřebné záruky nepředstírá atomickou lokální operaci. Zdroj přesunu se nesmaže před úspěšným ověřením a publikováním cíle. Bezpečný transport a politika přihlašování jsou součást návrhu konkrétního síťového providera.

### Povinné testy cest

- Lokální Unicode cesty pod i nad 260 UTF-16 jednotek na Windows; dlouhé legální komponenty, hluboké stromy a UNC dle dostupnosti prostředí.
- Dlouhé cesty na Linuxu podle schopností filesystemu, bez přeneseného limitu Windows.
- Zachování celé identity při zobrazení, historii, řazení, výběru, chybách a budoucích kopírovacích/ověřovacích operacích.
- Testovací vzdálený provider s cestami výrazně nad `MAX_PATH`, Unicode jmény, vlastními oddělovači a vlastní sémantikou kořene; odmítnutí nechtěné lokální normalizace.
- Chyba skutečného limitu providera nebo paměťového rozpočtu je viditelná a nedestruktivní, bez zkrácení či záměny cíle.

## 15. Platformní rozhraní a oddělené implementace

### Hranice kódu

Platformní vrstva se skládá z malé továrny/fasády `midnight-platform`, přenositelného
`midnight-platform-common` a samostatných `midnight-platform-windows`, `midnight-platform-linux`,
později `midnight-platform-macos`. Továrna pouze vybere/reexportuje implementaci pro
daný target; neobsahuje implementace filesystemu. Windows Win32 API a Linux
mountinfo/native logika se nikdy nesdílejí v jednom implementačním souboru.

Společný kód obsahuje jen skutečně přenositelné operace přes bezpečné Rust API,
pomocné datové struktury a doménové smlouvy. OS-specifické testy patří také do
samostatných souborů; stejné smluvní testy běží nad zvolenou implementací na každé
podporované platformě. Platformní crate se nekompiluje jako cizí implementace na
jiném targetu. UI ani engine neimportují Win32, `/proc`, konkrétní platformní
crate nebo `cfg` větve s OS logikou.

### Povinné služby první etapy

Ve `midnight-core` jsou objektově použitelná rozhraní:

| Rozhraní | Smlouva |
| --- | --- |
| `DirectoryService` | `read_directory(path, cancel, emit)` postupně vrací položky; `entry_metadata(path)` vrací metadata bez otevírání obsahu speciálních souborů |
| `VolumeService` | `list_volumes()` vrací navigační kořeny bez čekání na dotazy na kapacitu jednotlivých svazků |
| `StorageService` | `space_info(path)` vrací kapacitu; `storage_kind(path)` dává konzervativní LocalSsd/Rotational/Network/Unknown hint pro plánování |
| `FileSystem` | Agreguje povinné služby jako `DirectoryService + VolumeService + StorageService`; engine přijímá toto abstraktní rozhraní |
| `IconService` | Backend vrací omezené RGBA ikony podle typu; žádné otevírání zobrazených programů nebo UI I/O |
| `TerminalService` | Spustí výslovně zadaný příkaz v systémovém terminálu se zachyceným pracovním adresářem |
| `PropertyService` | Explicitní systémové vlastnosti nebo poctivý signál pro read-only metadata fallback |
| `LocationService` | Skutečné disky a podporovaná fyzická speciální umístění, písmena disků jen na Windows |
| `OpenService` | Pouze explicitní otevření skutečné položky: adresáře interně, běžné soubory přes systémové asociace; symlinky klasifikuje backend |
| `DesktopServices` | Agregát předchozích pěti desktopových rozhraní; oddělený od `FileSystem` |
| `WatchService` | Ne-rekurzivní systémové změnové hinty a vlastněná subscription; debounce, kontrola a polling patří enginu |
| `SettingsService` | Načtení/verzované atomické uložení typovaných preferencí pouze na backendu |

Každá OS implementace musí dodat tyto služby; může delegovat přenositelné části
na `midnight-platform-common`. Podpisy zachovávají bezeztrátové cesty, strukturované
chyby a kooperativní rušení. Synchronní lokální provider API smí engine volat
pouze na pracovních vláknech. Budoucí síťové providery mohou používat nativně
asynchronní I/O; lokální blokující bridge nesmí určovat jejich protokol ani nutit
síťové operace na UI vlákno.

### Služby přidávané spolu s příslušnou funkcí

- `CopyPublicationService`: práce s otevřenou identitou souboru, výhradní dočasné
  vytvoření, synchronizace, publikování bez přepsání a bezpečná výměna. Jednotlivé
  schopnosti jsou explicitní; chybějící záruka vrací `Unsupported` před mutací.
- `TrashService`: přesun do koše s výsledkem a dostupnými možnostmi obnovy.
- `MountService`: připojení a bezpečné odpojení zařízení včetně potřebné uživatelské interakce.
- Další schopnosti asociací a explicitního otevření rozšíří existující desktopové služby; žádné sestavování shellových příkazů z názvů souborů.

Tyto budoucí služby se dnes nenahrazují nefunkčními stuby ani falešným úspěchem.
Přibudou s implementací, deklarací schopností a smluvními testy dané funkce.
MacOS nebo další OS získá funkce implementací stejných smluv a průchodem
společnými i vlastními OS testy. FTP/FTPS/SFTP jsou nezávislí filesystemoví
provideři nad těmito hranicemi, nikoli varianty Windows implementace.

## 16. Styl zdrojového kódu

Závazný stručný dokument je `CODE_STYLE.md`: oficiální veřejný Rust Style Guide,
Rust API Guidelines a výstup `rustfmt`, cíl 100 znaků na řádek, čtyřmezerové
odsazení, idiomatické názvy a minimální veřejný povrch s dokumentovanými chybami.
Ručně psané zdrojové soubory mají nejvýše 100 000 bajtů; generovaný kód,
lockfile, třetí strany a média jsou výjimky. Počet řádků je vodítko pro rozdělení
odpovědností, nikoli další pevný limit. Velikost kontroluje
platformně oddělené `scripts/check-source-size.ps1` / `scripts/check-source-size.sh`
lokálně i v CI. Platformní oddělení, bezpečné API,
Unicode a asynchronní hranice jsou součást stylu a code review, nikoli jen formátování.

## 17. Multiselekce, dávky a společná fronta operací

### Výběr a neměnný požadavek

Každý panel má nezávislý kurzor, kotvu rozsahu a multiselekci. Návrh zahrnuje
Space/Insert, Ctrl+klik, Shift+rozsah a výběr všeho. Výběr je navázán na skutečné
identity položek a generaci výpisu, nikoli na indexy řádků nebo zkrácené názvy.
Velké rozsahy a materializace seznamu položek se zpracují v back-endu; UI nedělá
synchronní průchod statisíci soubory.

Při zadání operace vzniká neměnný snapshot výběru a cílového umístění. Back-end
z něj vytvoří plán; následná navigace, změna aktivního panelu či výběru nesmí
přesměrovat už zadanou úlohu. Před skutečným otevřením se revalidují dostupné
identity a podmínky operace. Snapshot výběru není snapshot obsahu měnícího se
souboru; pravidla stability zdroje a povinného ověření zůstávají platná.

### Jednotný model úloh

- `OperationRequest` zahrnuje `Copy`, `Move` a `Delete`; další typy se přidávají
  samostatnými handlery ve stejném back-endovém scheduleru.
- `EnqueueOperation(request_id, selection_snapshot, destination, policy)` má
  neblokující vstup a potvrzení `OperationAccepted { request_id, job_id }`, nebo
  konkrétní odmítnutí. UI nepředstírá zařazení před přijetím. Opakovaný request_id
  nezaloží omylem druhou destruktivní úlohu.
- V první verzi se deduplikační identita uchovává po dobu procesu i po dokončení
  úlohy. Obnova po restartu nesmí automaticky přehrát mutace; případná budoucí
  perzistentní fronta uloží request_id a přijetí transakčně společně.
- Fronta i autoritativní registr úloh patří enginu. Úloha přežije změnu/zavření
  panelu průběhu; není vlastněná dočasným dialogem nebo souborovým panelem.
- Během aktivní operace zůstává možné prohlížet oba panely, měnit výběr a zadat
  další Copy/Move/Delete. Kapacitně omezený vstup nikdy nezablokuje callback UI;
  při zaplnění vrátí jasné odmítnutí místo tichého zahození požadavku.
- Úlohy mají vlastní identitu, chyby, rozhodnutí o konfliktech a zrušení. Zrušení
  jedné úlohy neruší další úlohy ani navigaci. Na rozhodnutí uživatele čeká stavový
  automat, nikoli zablokované vlákno.

Základní stavy: `Queued`, `Planning`, `Running`, `WaitingForDecision`,
`WaitingForDependency`, `CancelRequested`, `Cancelled`, `Completed`,
`CompletedWithErrors`, `Failed`. Běžící kopie má navíc fáze `Copying`, `Syncing`,
`Verifying`, `Publishing`. Výsledek celé dávky a výsledky jednotlivých souborů
jsou oddělené; přeskočené či chybné soubory se nezamění za úplný úspěch.

### Průběh

Seznam operací ukazuje čekající i aktivní úlohy a u každé počet souborů,
aktuální soubor, zpracované bajty, fázi, dostupný odhad a chyby. Kopie zobrazuje
samostatně přenos a ověřování. Soubor se počítá jako dokončený až po úspěšném
ověření a publikování; poslední zapsaný bajt neznamená 100 % hotové operace.

Pro celé stromy se plán a součty získávají postupně, bez blokujícího úplného
předběžného skenu. Neznámý celkový počet/velikost používá neurčitý průběh;
nevymýšlí se falešné procento. Časté průběžné hodnoty se slučují na poslední stav
a UI se aktualizuje dávkovaně, ale přijetí, chyby a terminální výsledky jsou
doručované spolehlivě.

### Pořadí a bezpečný souběh

Nové zadání a provádění jsou nezávislé: přijetí nové úlohy nečeká na konec staré.
Počáteční scheduler používá konzervativní FIFO s nejvýše jednou aktivní mutující
operací; uvnitř jedné kopírovací operace smí běžet několik nezávislých souborů
podle rozpočtů v oddílu 8. Čekání operace na rozhodnutí zabírá logický slot, nikoli pracovní
vlákno; nové požadavky se nadále přijímají. Navigace má vlastní pracovní kapacitu.
Budoucí paralelní zpracování povolí jen nezávislé operace podle zdrojového/cílového
providera, zařízení a konfliktů, se stejnými sdílenými I/O limity; samotné spuštění
tisíců Tokio tasks není plánování. Nové kopírování/přesun/mazání lze zařadit i během
kopírování nebo jeho ověřování. Nezávislé soubory běžící operace mohou pokračovat,
pokud jejich výsledek nezávisí na čekajícím rozhodnutí; cílové identity a zámky se
při čekání znovu ověří podle oddílu 25.

Kolidující cesty, podstromy a zjistitelné aliasy mají chráněné pořadí. Zejména
nesmí souběžné mazání/přesun zdroje zneplatnit běžící kopii ani dvě úlohy
nekontrolovaně publikovat do stejného cíle. Při selhání kopie se navazující
destruktivní úloha nad stejnými daty automaticky nespustí; přejde do chyby nebo
čekání na nové vědomé rozhodnutí. Pravidla identity/kolizí určuje provider,
nikoli globální převod Unicode názvů na malá písmena.

Závislost chránící destruktivní krok je splněná pouze úplným `Completed`
předchozí operace, včetně ověření/publikování všech požadovaných souborů.
`CompletedWithErrors`, `Failed` nebo `Cancelled` ji nesplňují.

Přesun mezi filesystémy je ověřená kopie a teprve poté odstranění zdroje.
Přesun v rámci jednoho filesystemu může použít jeho bezpečný rename kontrakt.
Mazání má vlastní deklarovanou politiku koše/trvalého odstranění. Rušení nevrací
již dokončené destruktivní kroky zpět a UI takový rollback neslibuje.

### Povinné přejímací testy

- Multiselekce a dávka používají přesně zachycené položky i po přepnutí panelu,
  navigaci, změně řazení nebo změně aktuálního výběru.
- Zablokovaná kopie nebrání přijetí další kopie/přesunu/mazání ani odezvě UI;
  všechny potvrzené požadavky mají dohledatelný JobId a správné pořadí.
- Průběh zahrnuje ověřování; poškození jediného cíle nesmí dát celé dávce stav
  úspěchu. Duplicity request_id, zaplnění front a zahlcení progress událostmi
  nezpůsobí zdvojenou operaci nebo ztrátu koncového výsledku.
- Kolidující operace a závislosti jsou serializované; selhání předchozí kopie
  nezpůsobí automatické smazání jediného platného zdroje.
- Zrušení jedné dávky je izolované; čekání na konflikt neblokuje UI/vlákno;
  krátké zápisy, plný disk, změna zdroje a výpadek serveru mají konkrétní výsledky.

## 18. Sjednocený rozsah a aktuální ovládání

Tato tabulka rozlišuje hotový základ od právě doplňovaných částí a budoucích etap.
Provedené testy nejsou náhradou za dosud neprovedená měření či nasazení na Mint.

| Oblast | Stav / závazek |
| --- | --- |
| Rust/Slint/Tokio, jeden proces a oddělené crates | Implementováno; UI nezná konkrétní engine ani OS API |
| Windows x64, Linux | Implementováno a testováno na Windows a WSL Ubuntu 24.04; samostatná kvalifikace Mint zbývá |
| Unicode, raw identity, cesty bez aplikačního limitu | Závazné; testy včetně dlouhých Windows cest a ne-UTF-8 Linux názvů |
| Dva panely, svazky, historie, postupná metadata, multiselekce | Implementováno; řádky jsou virtualizované |
| F1–F10, editor příkazů, vlastnosti, ikony, umístění, `..` | Implementováno; rozsah funkčních tlačítek níže |
| Výpočet velikostí a přepínání směru řazení | Implementováno včetně sdíleného plánování SSD/HDD/síť/Unknown z oddílu 19 |
| Automatická obnova a zachování rozpracovaného UI | Implementováno; nativní události i fallback polling |
| Odstranění horního pruhu, témata, hustota, ukládání preferencí | Implementováno podle oddílu 21; shell picker je na všech aktuálně podporovaných platformách |
| Kopírování/přesun/mazání a jejich společná fronta | Následující etapy podle uživatelem zvoleného read-only rozsahu; ověření nelze obejít |
| FTP/FTPS/SFTP, archivy, macOS | Připravené hranice providerů/platformních služeb; implementace později |
| Vestavěný viewer a obecné VFS | Finalizovaný návrh dynamických pluginů: text/source, nativní OS provider, ZIP/GZIP/TAR.GZ/7z; implementace následuje podle oddílů 22–23 |

| Vstup | Chování |
| --- | --- |
| Tab | Přepne aktivní souborový panel |
| Levé kliknutí / Ctrl+klik | Kurzor a aktivní panel / přepnutí výběru položky; samotný výběr nic nespouští |
| Enter / levý dvojklik bez modifikátorů | Vstup do adresáře nebo otevření běžného souboru výchozí systémovou aplikací; odkazy na adresáře zůstávají v panelu |
| Backspace / dvojklik na `..` | Navigace k rodiči; `..` není skutečný soubor ani součást výběru |
| Space / Insert | Multivýběr a posun kurzoru; Space navíc počítá velikost skutečné složky na pozadí |
| Alt+Left / Alt+Right, Ctrl+R | Historie / nový výpis aktivního umístění |
| Kliknutí na záhlaví Name/Ext/Size/Modified | První volba kritéria vzestupně; opakované kliknutí přepíná asc/desc; šipka ukazuje směr. Ext řadí podle přípony stejným backendovým příkazem jako nabídka Extension a Ctrl+F4. Name a Ext sdílejí oblast názvu; řádky zachovávají celý název souboru. Kliknutí aktivuje příslušný panel. |
| Ctrl+Shift+S | Výpočet vybraných složek, nebo všech složek bez výběru; opakování ruší. Podporuje nativní složky a připravený seznam archivu. |
| Alt+Enter, také Ctrl+Alt+Enter | Vlastnosti vybraných položek, jinak skutečné položky pod kurzorem; samotné `..` nic nespouští |
| Ctrl+[ / Ctrl+] | Vloží celou cestu levého/pravého panelu jako samostatný bezpečně citovaný argument |
| Enter / Run v příkazovém editoru | Explicitně zadaný příkaz spustí v externím systémovém terminálu, cwd se zachytí při zadání |
| Výběr vlevo od příkazového editoru | Windows PowerShell/cmd; POSIX Terminal/Terminal (sudo). Běžná Windows volba se ukládá, zvýšený POSIX režim pouze do konce relace |
| Shift+písmeno disku | Pouze Windows a zaměřený seznam souborů; v editorech zůstává běžné psaní |
| Lišta disků a míst nad každým panelem | Cílí pevně na svůj panel; disk obnoví jeho zapamatovaný adresář, speciální místo otevře přesnou cestu |
| Options / menu | Volba tématu a hustoty; About je v menu, nikoli v samostatném horním řádku |

Spodní lišta zachovává referenční pořadí F1 Help, F2 Rename, F3 View, F4 Edit,
F5 Copy, F6 Move, F7 Create directory, F8 Delete, F9 User menu, F10 Menu.
Dosud neimplementované akce jsou skutečně zakázané; klávesy neobcházejí stav
tlačítek. F10 otevírá menu, ukončení je jeho samostatná položka. Příkazový editor
je výslovná akce uživatele a může přes systémový shell měnit soubory; read-only
rozsah se týká interních souborových operací manažeru.

Windows používá důvěryhodné absolutní systémové cesty pro PowerShell i cmd.
PowerShell předává Unicode přes `-EncodedCommand`; cmd přes přesný Unicode
command line s vypnutým AutoRun a delayed expansion. Linux spouští podporované
systémové terminály se samostatnými argumenty shellu; sudo pouze při výslovně
zvoleném Elevated režimu. Podle upřesnění uživatele z 7. listopadu 2026 se názvy
a cesty vkládají jako přesný text bez přidaných uvozovek, escapování a oddělovacích
mezer. Skutečné mezery v názvu zůstávají. Vložení je pouze editace; celý příkaz
interpretuje zvolený shell až po explicitním Enter/Run. Řídicí znaky a názvy,
které nelze přesně reprezentovat, se odmítnou bez ztrátové náhrady.

Cmd také nesmí potichu změnit UNC pracovní adresář na lokální:
nepodporované cwd vrátí chybu s doporučením PowerShellu.

Sudo komunikuje s heslem přímo v externím terminálu, nikdy přes UI nebo `-S`.
Standardní režim je výchozí při novém spuštění; Elevated se neukládá. Běžná volba
PowerShell/cmd je součást UserPreferences. Připravené POSIX UI není tvrzení, že
už existuje macOS launcher; ten musí dodat samostatný budoucí adaptér.

Systémové ikony vznikají na backendu podle typu/přípony, ne extrakcí ze spuštěného
či zobrazeného programu. Přenos jsou omezené RGBA pixely a UI kešuje Slint Image.
Chybějící ikona má generovaný fallback. Windows používá typové Shell/stock ikony,
Linux zatím omezené instalované PNG motivy. Vícenásobné či nepodporované nativní
vlastnosti mají read-only metadata fallback, nikdy implicitní otevření souboru.

Scroll zachovává nativní dojezd prstu/touchpadu. Diskrétní kolečko v souborovém
panelu převádí na krátkou pixelovou animaci a po klidu zarovná řádek. Kliknutí,
klávesnice či změna cíle zruší zbylý pohyb; metadata nevracejí viewport ke kurzoru.

## 19. Velikosti složek, férovost a typ úložiště

Součet používá logické délky běžných souborů, nikoli alokované bloky. Explicitní
symlinky/junctions a speciální soubory se nepřecházejí. Průběžná, zrušená nebo
chybová hodnota je zřetelně neúplná, ne úspěšná nula. Sčítání kontroluje přetečení.
Průchod je živý odhad měnícího se stromu; path-based read-only API neslibuje
atomický snapshot nebo bezpečnostní záruku pro budoucí destruktivní průchody.

Hotová část používá rušení po položkách/adresářích, průběh nejvýše přibližně
10krát za sekundu na kořen a omezenou frontu cest (16 MiB na průchod). Vyčerpání
rozpočtu dává explicitně neúplný výsledek. Jednotlivá cesta nemá pevný limit.
Řazení je samostatný backendový `Resort` nad přijatým snapshotem: zachová ID
položek a hotové bezchybné velikosti. Neúplné hodnoty se nepoužijí jako přesná
velikost. Refresh načte skutečná aktuální data a staré výpočty zneplatní.

Nově požadovaná politika plánování:

| Úložiště | Aktivní I/O pro výpočet | Důvod |
| --- | --- | --- |
| Prokazatelně lokální SSD / NVMe | Malá omezená dávka, výchozí strop 4 | Menší složka může dokončit souběžně s velkou; nezavádět úlohu pro každou složku/soubor |
| Rotační HDD | 1 | Omezit souběžné seeky; férově střídat omezené úseky aktivních průchodů |
| Síťový disk, UNC, NFS/SMB, vzdálený provider | 1 | Paralelní požadavky mohou zahltit server nebo zvýšit latenci |
| Neznámé, virtuální či neprůkazné médium | 1 | Při nejistotě nepředpokládat SSD podle naměřené rychlosti nebo cache |

Detekce patří platformním službám a běží pouze na backendu. Windows ověřuje
síťový charakter a vlastnosti storage zařízení; Linux mount/device informace a
rotaci lokálního blokového zařízení. Nedostupná či nejednoznačná informace vede
k `Unknown`. Cesta nad lokálním SSD může vést přes další mount na jiné médium;
nejistá či smíšená topologie nesmí automaticky získat agresivní SSD rozpočet.

Rozpočet je sdílený mezi panely, ne samostatné čtyři workery na každý panel.
Zejména dva panely nad stejným síťovým serverem nesmějí násobit jeho zátěž.
Průchod má omezený počet rozpracovaných složek a férové kratší pracovní úseky;
nepoužije neomezeně vláken, otevřených adresářů ani úloh v Tokio poolu. Jedna
neukončitelná systémová operace může držet svůj slot; okamžité zastavení OS I/O
se neslibuje. Navigace, watchery a render mají oddělenou pracovní kapacitu.

Přejímací testy: zablokovaná velká složka na SSD nebrání malé v aktivní dávce,
strop souběhu se nepřekročí, HDD/Unknown/síť nikdy neprovádí více I/O současně,
dva panely sdílejí síťový rozpočet, detekční chyba volí konzervativní režim a
zrušení propustí čekající práci bez falešných terminálních výsledků.

## 20. Watchery, dávkovaná obnova a skutečně viditelný snapshot

Každá platforma implementuje `WatchService` v samostatném souboru. Aktuální
Windows adaptér používá nativní directory notifications a Linux inotify přes
notify 8.2.1; macOS musí dodat svůj adaptér. Sledují se okamžité obsahy viditelných
adresářů. Create/delete/rename/změna dat či metadat/error/overflow jsou hinty
k opětovnému výpisu. Read/access události se ignorují, aby samotný výpis
nevyvolával nekonečnou smyčku.

Engine uchovává jen poslední cíl a dirty stav každého panelu. Obnova proběhne po
1 sekundě klidu, nejpozději po 3 sekundách nepřetržitých změn. Dva omezené
backendové workery vlastní registrace, rušení i polling. Každých 15 sekund
porovnají skutečná jména, typy, délky souborů a časy změn. Nezměněná kontrola
refresh nevyvolá. Obnovení dříve nedostupných metadat jej vyvolá právě tak,
aby se nové hodnoty dostaly do UI. Vypočtený součet složky není fyzická délka
adresáře a do porovnání se nezaměňuje.

Polling zůstává aktivní i po úspěšné nativní registraci: síťové filesystémy a
některé Windows overflow/read-error případy nemusí notifikaci doručit. Hints
neslouží jako důkaz integrity. Odpojené médium/OS chyby mohou obnovu oddálit.

UI potvrzuje skutečně přijatý výpis příkazem `PaneCommitted(VisibleDirectory)`
se sdíleným `Arc` snapshotem. Výpis, který worker dokončil, ale UI jej jako starý
ignorovalo, se nesmí stát autoritativním viditelným stavem. Omezenou vstupní
frontu jistí krátké opakování potvrzení po 100 ms. Hint nese panel/generaci;
zastaralý hint ani probíhající uživatelskou navigaci nepřepisuje.

Obnova stejného umístění připravuje nová data za stále použitelnými starými
řádky a výsledek vymění najednou. Zachová surové identity výběru/kurzoru,
viewport, aktivní panel a rozepsaný příkaz včetně caret/fokusu. Při chybě zůstává
poslední použitelný snapshot; polling později obnovu zkusí znovu. Sledování se
ukončuje na backendu a plná výstupní fronta nesmí bránit shutdownu.

## 21. Témata, hustota ovládání a uživatelské nastavení

Horní branding řádek Midnight Explorer/About se odstraní. Tlačítko **Options** bude u příkazového
řádku a dostupné také v menu. Okno nastavení obsahuje nezávislé výběry:

| Volba | Varianty |
| --- | --- |
| Téma | Blue (dosavadní modré, výchozí), Dark (neutrální tmavé), Light (světlé) |
| Hustota | Desktop / mouse (kompaktní), Touch (větší rozestupy a dotykové cíle) |
| Font | Nainstalovaná rodina nebo výchozí font aplikace |
| Základní velikost | Nezávislá hodnota Desktop a Touch,8–32 logických px |

Společná Slint Theme vrstva poskytne sémantické barvy pro pozadí, panely, text,
neaktivní/aktivní stav, výběr, fokus, chyby a zakázané ovládání. Light přepne také
schéma standardních Slint widgetů; nestačí změnit jen pozadí seznamu. Všechna
témata zachovají čitelný výběr, aktivní panel a indikaci chyby. Statické toolbar
ikony jsou vložené prostředky, aby vykreslování neotvíralo soubory z UI.

Desktop používá22px řádky a úsporné rozestupy. Touch používá minimálně
40 logických px pro souborové řádky a hlavní ovládací cíle, větší výšky editorů,
tlačítek a padding. Jedna sada metrik řídí render i výpočty scrollu/reveal/hit
testů; kód nesmí předpokládat pevnou výšku po přepnutí hustoty. Změna tématu ani
hustoty nesmí měnit aktuální cesty, výběr, operace nebo rozepsaný příkaz.

Okamžitý náhled je čistá UI operace. Options používá **OK / Cancel**:
OK ověří změny na všech stránkách, uloží je asynchronně a zavře okno až po
úspěšném zápisu. Cancel, Escape a křížek zahodí celou rozpracovanou relaci včetně
náhledu tématu, fontů, hustoty a již přiřazených klávesových zkratek. Při chybě
zůstane dialog otevřený. Automatické ukládání polohy, karet a historie nesmí
uložit nepotvrzené náhledy. Podrobnosti a souběhy popisuje
[transakce nastavení](2026-11-08-options-transaction.md). Perzistence běží v backendu:
`SettingsService` ukládá typované `UserPreferences` ve verzovaném souboru
(v3; migrace v1/v2). Windows použije uživatelské LocalAppData, Linux absolutní XDG_CONFIG_HOME
nebo ~/.config; platformní rozhodnutí zůstávají v oddělených OS souborech.
Konfigurace neobsahuje spustitelné příkazy nebo přístupové údaje.
Obsahuje také běžnou volbu Windows shellu; zvýšený POSIX režim se neukládá.

Načítání/zápis/úklid běží na workeru, ne v UI. Chybějící soubor znamená výchozí
hodnoty; chybná či nepodporovaná data vrací chybu bez přepsání původního souboru.
Čtení je omezené velikostí, ověřuje otevřený běžný soubor a nesmí uváznout na
FIFO, zařízení či odkazu. Uložení používá vlastní výhradně vytvořený dočasný
soubor, dokončení zápisu/synchronizaci a atomické publikování; nikdy nejprve
nevymaže existující nastavení. Linux používá soukromá oprávnění.

Požadavky mají monotónní `PreferencesRevision`. Opožděné načtení nesmí přepsat
novější uživatelskou volbu; starší uložení nesmí zvítězit nad novějším. Engine
serializuje zápisy a drží nejvýše jeden běžící a poslední čekající stav. Selhání
uložení nezruší aktuální náhled a zobrazí možnost opakování. Po zavření okna
proběhne omezené dokončení posledního přijatého uložení (cílově do 2 sekund);
blokující OS volání ani pád napájení nedostávají falešnou záruku durability.

Testy zahrnou všech šest kombinací tématu/hustoty, skutečný render světlých i
tmavých ploch, touch hit test a scroll, zachování fokusu/caret, závod load/save,
rychlou sérii změn, chybu a opakování uložení, změnu a okamžité zavření, poškozený
soubor, odkazy/FIFO/zařízení a Unicode i dlouhou cestu ke konfiguraci.

Příkazový draft je při prvním psaní nebo vložení cesty navázaný na konkrétní
shellovou gramatiku. Opožděně načtené nastavení nesmí gramatiku neprázdného draftu
změnit. Přepnutí PowerShell/cmd s neprázdným příkazem se odmítne s pokynem nejprve
vyčistit řádek; již citovaná cesta s `&` se nesmí změnit ve spustitelnou syntaxi.
Přepnutí Standard/sudo v rámci POSIX gramatiky je výslovná změna oprávnění,
stále vyžaduje Enter/Run a nezapisuje zvýšený režim do konfigurace.

## 22. Vestavěný viewer: dynamické pluginy pro text/source a později Markdown

Aktuální [finální plugin kontrakt](2026-11-05-dynamic-viewer-filesystem-plugins-design.md)
stanoví samostatné knihovny v `viewers/`, discovery při startu, verzované C ABI,
backendové leases a společný Slint renderer. První plugin poskytne plain text
a zvýraznění syntaxe známých zdrojových jazyků. Odebraný plugin odebere příslušnou funkci;
F3 se aktivuje až se skutečnou implementací. UI neobsahuje skrytý parser fallback.

Čtení, dekódování, indexování, syntax, hledání, velké výběry a cleanup patří
backendu. UI vykreslí omezené UTF-8 stránky a sémantické spany se source map,
generation a revision. Unicode shaping, výběr a bidi zachovají společný kontext
řádku; jednotlivé tokeny se nesmějí rozpadnout do nezávislých Text widgetů.
Sekvenční stream nepředstírá levný random access, velký soubor se nenačítá celý.

Markdown přijde jako další plugin s vyjednaným block modelem. Zachová zákaz
aktivního HTML/scriptu a automatického stahování i hostitelskou LinkPolicy.
Podrobnosti obsahových kontraktů, Unicode, cache, source změn a reference
Salamanderu zůstávají v [původním viewer/VFS návrhu](2026-11-04-viewer-vfs-design.md).

## 23. VFS a migrace na platformní nativní pluginy

Každý filesystem včetně nativního je samostatný dynamický plugin ve
`filesystems/`. Windows, Linux a později macOS mají oddělené implementace a testy.
První archivní balíčky jsou ZIP, GZIP, TAR (také TAR.GZ/TGZ) a 7z; procházení
členů a F3 jsou read-only. GZIP zpřístupní jeden logický stream, TAR.GZ strom;
TAR má vlastní decoder helper a nezávisí na přítomnosti GZIP pluginu.

Současné PathBuf rozhraní se migruje přes adaptér na provider registry,
MountId/NodeRef a content broker. Provider vlastní namespace, parent/root,
case pravidla, persistent address i raw identity. Display text není OS cesta;
není žádný aplikační MAX_PATH. Capabilities pravdivě rozlišují čtení/seek/watch,
metadata a budoucí mutace. Odebraný provider ponechá karty/záložky nedostupné,
neztratí jejich uložené adresy. Chybějící native plugin odebere lokální browsing;
vestavěné host služby nastavení, terminálu, ikon a loaderu zůstanou funkční.

Archivy používají backing leases, bounded index/dekompresi, bezpečné opaque
identity členů a watch invalidaci. Neextrahují automaticky cesty ani odkazy.
Budoucí extraction/copy vyžaduje společný BLAKE3 destination readback a bezpečné
publish; CRC archivu není náhrada. Registry zůstává budoucí Windows plugin
s klíči a typovanými hodnotami, vzdálené zdroje dalšími capability implementacemi.

Plný kontrakt, balení, životnost, bezpečnost a přejímací matice jsou v
[novém návrhu dynamických pluginů](2026-11-05-dynamic-viewer-filesystem-plugins-design.md).
Nativní DLL/SO jsou důvěryhodný kód s právy aplikace, nikoli sandbox. Knihovny
zůstanou načtené do ukončení procesu, instalace/odebrání se uplatní po restartu.
Pluginy jsou v jednom Git repozitáři, ale mají samostatné Cargo projekty,
buildy a vlastní testy/CI. Sdílí pouze veřejné
API/SDK, ne interní app crates. Vývoj, build, test ani instalace pluginu nesmějí
upravovat zdrojáky hlavní aplikace; rozšíření API je samostatná změna hostitele.

## 24. Opakované sudo a životní cyklus terminálové relace

Podrobný návrh je v [návrhu terminálových relací](2026-11-04-terminal-sessions-design.md).
Automatické znovupoužití není součást hotového launcheru: aktuální spuštění
vytváří nové externí okno/TTY. Ponechaný interaktivní root shell lze používat
ručně, ale editor manažeru do něj další příkazy automaticky neposílá. Návrh
tento rozdíl výslovně nezaměňuje za fungující cache.

Cílové řešení drží jeden **neprivilegovaný** broker ve stejném externím terminálu.
Každý výslovný příkaz projde skutečným systémovým sudo v téže relaci, čímž lze
využít systémovou cache podle místní politiky. Aplikace neukládá heslo, nepoužívá
`sudo -S`, nemění sudoers/NOPASSWD a neobnovuje oprávnění neomezeným periodickým
`sudo -v`. Systémový timeout a nové výzvy k ověření zůstávají respektované.

Příkazy a status používají vlastní omezený IPC kanál oddělený od nedůvěryhodného
výstupu terminálu. Broker i aplikace se ověří kernelovou identitou procesu/UID
a životností původního procesu; samotný user-owned socket či UUID ve veřejném
argv nestačí. Bez prokazatelného bezpečného bootstrapu konkrétního terminálu
adapter nesmí slibovat reuse. Heslo jde přímo mezi uživatelem, terminálem a sudo.

Další Enter smí poslat příkaz jen do idle relace; nesmí jej vstříknout do právě
běžícího editoru, programu nebo password promptu. Obsazená relace vrátí poctivý
Busy stav, nevytváří neomezenou frontu root příkazů. Operace se nezkouší vykonat
nejprve neinteraktivně a při neúspěchu podruhé: nenulový exit může být výsledkem
už vykonaného příkazu, nikoli chybějící autentizace.

Relace má explicitní zavření, omezenou nečinnost a vazbu na životnost aplikace.
Reused session není nový Child a nesmí spotřebovávat nové sloty/reaper záznamy.
Při odpojení během příkazu se rozlišuje neznámý výsledek od potvrzeného dokončení;
retry nesmí zdvojit destruktivní příkaz. Systémová globální sudo cache se
automaticky nemaže pomocí `-K`; dopady revokace závisejí na místní sudoers politice.

## 25. Konflikty při kopírování/přesunu a potvrzení přepsání

Jde o povinnou součást budoucích zápisových operací, nikoli povolení nyní aktivovat
neověřenou F5/F6 implementaci. Výchozí politika je nepřepisovat. Existující cíl
vyvolá strukturovaný konflikt a úloha přejde do `WaitingForDecision`; UI ani
backendový worker nečeká synchronně na dialog a další operace lze zařazovat.

Dialog porovná zdroj a cíl vedle sebe:

- název a úplnou surovou cestu / provider-owned reprezentaci bez změny identity;
- provider, typ položky (soubor/složka/odkaz/jiný objekt) a případnou kolizi typů;
- přesnou známou velikost v bajtech i čitelnou jednotku, neznámou hodnotu výslovně;
- datum a čas změny včetně časové zóny/rozlišení podle dostupných metadat;
- podle schopností další užitečná metadata (např. read-only stav), bez předstírání
  znalosti úplné rekurzivní velikosti složky nebo hash shody jen ze stejné délky.

Akce: **Přepsat / Přeskočit / Jiné jméno / Zrušit**. Případné **pro všechny tyto
konflikty** je výslovná politika aktuální dávky s jasným rozsahem, ne trvalý
globální souhlas pro nové úlohy. Kolize soubor/složka/odkaz a sloučení adresářů
mají vlastní schopnosti a rozhodnutí; prosté přepsání souboru je nesmí potichu
proměnit v rekurzivní mazání nebo následování odkazu.

Konflikt nese JobId, verzi rozhodnutí a zachycené identity/metadatový stav.
Odpověď na starý nebo už vyřešený dialog se odmítne. Před publikováním se cíl
znovu ověří; změněná identita nebo stav vyvolá nový konflikt, ne použití starého
souhlasu na jiný soubor. Hodnoty v dialogu se získávají na backendu a nesmějí být
jen neověřenou kopií zastaralého panelu.

Při přepsání se nejprve dokončí vlastní dočasný cíl, synchronizace a nezávislý
kontrolní readback s délkou a kryptografickým hashem. Existující cíl se předem
nemaže ani netruncuje. Teprve potom se použije deklarované/testované atomické
publikování či bezpečná výměna daného provideru. Chybějící záruka znamená
Unsupported, ne destruktivní fallback. U přesunu mezi filesystémy zůstává zdroj,
dokud cílová kopie není ověřená a úspěšně publikovaná; rename v rámci filesystemu
také musí respektovat kolize a platný souhlas.

Testy zahrnou všechny volby a jejich rozsah, metadata/neznámé velikosti/Unicode,
rozdílné typy a odkazy, vznik či výměnu cíle během dialogu i během kopie,
zastaralou odpověď, chybu/rušení po ověření před publikací a zachování původního
cíle při selhání. Datum ani stejná velikost nenahrazují ověření obsahu.

Atomická výměna sama o sobě není podmíněná výměna potvrzené identity. Provider,
který slibuje ochranu proti nahrazení cíle po poslední kontrole, musí mít zvláštní
schopnost `ConditionalOverwrite(expected_identity_or_version, staged_target)`
s definovaným linearizačním bodem, případně skutečně výhradní namespace kontrakt.
Pouhé stat následované rename/ReplaceFile tento slib nesplňuje. Bez této
schopnosti je silné podmíněné přepsání Unsupported; případná slabší politika
„nahradit aktuální položku na této cestě“ musí být zvlášť pojmenovaná a výslovně
zvolená, nikdy tichý fallback. Test musí změnit cíl i po poslední revalidaci těsně
před publikováním a prokázat, že nový soubor nepřepsal starý souhlas.

Implementační doplněk k oddílu 25: [nativní transfer kontrakt](../../developer/TRANSFERS.md)
odděluje výslovné „Replace · keep original“ od silné podmíněné výměny identity.
Původní objekty zůstávají v soukromém recovery adresáři; částečný commit nikdy
neopravňuje smazat zdroj. Merge a Replace all mají nezávislý rozsah jediné úlohy.
Nově vzniklý či změněný cíl vyžaduje nové potvrzení i po Replace all. Dialog
ukazuje typ, úplnou cestu, bajty a zachycený UTC čas; ve větším okně porovnává
zdroj a cíl vedle sebe. Jobs poskytuje omezené dočasné podrobnosti obnovy, nikoli
trvalý katalog. F6 a úplné zachování metadat mají samostatné navazující kontrakty.

## 26. Kompaktní ovládání a tooltipy

Trvalá vysvětlující nápověda nemá zabírat místo souborům a příkazům. Spodní
pruh s připomínkami zkratek, nápověda v prázdném příkazovém editoru a text ACTIVE
se odstraní; zvýraznění aktivního panelu, chyby a dostupná
nápověda F1 zůstávají. Navigační nástroje, hlavičky řazení, systémová umístění,
nastavení a příkazové ovládání mají skutečné tooltipy i přístupné popisky.

Volba shellu je malé ikonové tlačítko úplně vlevo, bezprostředně před příkazovým
editorem. Tooltip vysvětlí aktuální PowerShell/cmd nebo Terminal/(sudo), pracovní
adresář a použití. Rozbalená nabídka zachová celé čitelné názvy, ovládání
klávesnicí a rozlišení režimu sudo. Přepnutí samo nic nespouští; ochrana gramatiky
rozepsaného příkazu nadále platí. Aktuální cwd zůstane vedle editoru jako celý
read-only prompt bez výpustky a pevného omezení šířky. Výjimečně dlouhá cesta
má vodorovný posuvný výřez; příkazové ovládání zůstává v jediném řádku.
Prompt není součástí spouštěného příkazu.
Dotyková varianta používá po požadavku na zmenšení40px základní cíle.

Profil Desktop/mouse je výrazně hustší:22px řádky,24px ovladače,14px ikony,
6px okraje a4px mezery. Dotykový profil má40px řádky/ovladače,48px hlavní lišty,
10px okraje,6px mezery a20px ikony. Ikony souborů a složek se centrují podle výšky řádku. Desktopová
tlačítka disků mají46px s krátkým `C:`, speciální místa24px ikonové cíle s celým
popiskem v tooltipu; při dotyku zůstávají celé názvy. Rozbalovací šipka je SVG,
aby nezávisela na dostupnosti symbolu v systémovém fontu.

Options nastavuje `close-policy: no-auto-close`: kliknutí uvnitř ani mimo dialog
nesmí zahodit nebo uložit změny. Test skutečných vstupů musí projít náhledem
Dark→Touch→Light, OK s potvrzením backendu, zrušením přes Cancel/Escape/křížek
a novým otevřením. Samotné zavolání callbacku nebo screenshot to neověřuje.

Tlačítka disků zůstávají krátká. Tooltip obsahuje na Windows skutečný název
svazku a volné místo; Linux a budoucí macOS nepotřebují umělé písmeno nebo název
svazku, zobrazí kapacitu. Rozbalovací nabídka zobrazuje vedle písmene/kořene také
název, pokud jej platforma poskytuje, a volné místo. Šířka rozbalené nabídky je
nezávislá na zavřeném ovladači. Neznámé, načítané nebo nedostupné údaje mají
výslovný stav; chyba se nesmí změnit na zdánlivých 0 B volného místa.

`VolumeService::volume_label(path)` má výchozí `Ok(None)`; Windows poskytne
vlastní implementaci v samostatném souboru s dynamickými UTF-16 buffery, bez
MAX_PATH. `VolumeDetails` odděluje výsledek názvu a kapacity, aby dílčí chyba
nezrušila druhý údaj. `VolumeDetailsUpdated` nese ID i původní nativní kořen.
Oddělený backendový řadič ponechá rychlý výpis kořenů nezávislý na dotazech na
jednotlivé disky; metadata mají dva skutečně obsazené pracovní sloty, deduplikaci,
cache a pravidelnou obnovu přibližně po 45 s. Pomalé síťové a odpojené svazky
nesmějí držet kapacitu navigace; probíhající blokující OS volání si ponechá svůj
permit do skutečného návratu. Zastaralé výsledky odstraněných kořenů se zahodí.

Kapacita Windows používá vlastní `GetDiskFreeSpaceExW` s dynamickým zjištěním
kořene a volným místem dostupným aktuálnímu uživateli včetně kvót; původní
fixed-buffer cesta knihovny fs2 se pro Windows nepoužívá. Chybějící cesta zůstává
chybou. Dotazy na název i kapacitu potlačí dialogy nedostupných médií pouze na
svém pracovním vlákně a předchozí režim vždy obnoví. Expirace cache má vlastní
deadline, aby periodický tick nezměnil 45s interval na 90s.

Otevřená nabídka disku se zruší při změně přesné posloupnosti ID a nativních
kořenů, i když se dvě různé nevalidní Unicode cesty zobrazují totožně. Aliasy
jako Home uvádějí v nabídce a tooltipu svou skutečnou cestu.

UI při hoveru pouze čte cache, nevolá filesystem ani systémová API. Názvy jsou
obyčejný Unicode text, nikoli Markdown, HTML, příkazy nebo identita cíle. Nativní
cesty se nikdy nenahrazují zobrazeným textem. Referencí je Salamander
`src/toolbar6.cpp:241` a `src/drivelst.cpp:2561`, `:2621`; jeho synchronní/ANSI
postup se nepřebírá. Podrobný návrh je v
`2026-11-04-compact-tooltips-design.md`.

## 27. Karty, uložené cesty a další upřesnění ovládání

Aktuální produktová verze je **0.1** (`0.1.0` v Cargo SemVer). Titulek nativního
okna je `Midnight Explorer 0.1 — File Manager`; hodnota se odvozuje z verze balíčku.

Každý panel má samostatné karty cest. Výchozí jméno karty vždy odpovídá názvu
aktuální složky a při navigaci se změní; kořen zobrazí svou cestu. Celá cesta je
v tooltipu. Ctrl+T přidá kartu, Ctrl+W zavře aktuální, Ctrl+Shift+Tab volí předchozí,
Ctrl+Shift+1…9 konkrétní kartu. Ctrl+Tab fokusuje příkazový řádek. Poslední karta
zůstává otevřená; limit je 32 na panel. Karty a aktivní pozice se ukládají mezi běhy.
Levý dvojklik do volného místa za kartami přidá kartu se stejným aktuálním umístěním
daného panelu. Dvojklik na existující kartu, pravé a prostřední tlačítko další kartu
nevytvářejí; prostřední tlačítko zachovává zavření karty.
Každá karta má vlastní historii, řazení a kurzor pro aktuální běh.
Neaktivní karty nedrží velké výpisy ani skeny; přepnutí načte čerstvý obsah.
Opožděné backendové výsledky nesmějí přepsat jinou kartu.

Uložené cesty podle Salamanderu mají jméno, nezměněnou nativní cestu a volitelnou
číselnou zkratku. Nabídka hvězdičky umožní přidání, otevření a odebrání. Až64
záložek,10 číselných slotů; správce přiřazuje, Ctrl/Shift+číslo otevírá,
Alt navíc cílí do druhého panelu. Identita není odvozena z názvu ani zobrazené
cesty. Uložené záložky sdílí atomické asynchronní nastavení verze5; nativní
ne-Unicode názvy mají bezeztrátový platformní kodek. Souhrnný limit nastavení
4MiB chrání paměť, není limitem délky cesty; nadlimitní uložení selže bez zkrácení.

Běžné úspěšné zprávy nezabírají stálý footer. Každý panel má nově kompaktní
metadata řádku pod kurzorem, kopírovatelná kliknutím. Spodní tlačítka mají pro myš základ22px, dotyk40px. Probíhající
velikosti složek ukazují kontrastní barvu a při známém dílčím součtu `≈`, bez `!`.
Options doplňuje systémový font a nezávislé základní velikosti Desktop/Touch
8–32px, výchozí11/15; ostatní text a ovladače se odvozují od těchto hodnot.
Výběr fontů je virtualizovaný, seznam připravuje a cachuje backend.

Ctrl+Enter připojí jméno aktuální položky na konec příkazového editoru;
Ctrl+P vloží cestu aktivního panelu. Použije se bezpečné citování podle shellu,
nikdy automatické spuštění. Salamander zkratky pro implementované funkce,
pixelové rolování s jemným dosednutím na řádek a ikony jednotlivých disků
upřesňuje [podrobný návrh](2026-11-04-navigation-and-appearance-design.md), včetně
testů, ukládání nastavení a omezení této etapy.

Zkratky výběru: Ctrl+A vybere soubory hlavní úrovně, Ctrl+Shift+A také složky.
Ctrl+D odznačí pouze soubory a zachová složky; Ctrl+Shift+D zruší celý výběr.
Ctrl+I invertuje pouze soubory aktivního panelu (také uvnitř archivu), se zachováním
výběru složek a odkazů na složky. Hromadné výpočty běží v backendu; UI čte sdílený neměnný
bitový snapshot a potvrzuje odpovídající kontext a revizi výběru. Editory si
zachovají běžné Ctrl+A pro text. Úplná aktuální mapa je v
[KEYBOARD_SHORTCUTS.md](../../KEYBOARD_SHORTCUTS.md).

## 28. Plynulý výpočet velikostí a sjednocená navigační lišta

Aktivní výpočet velikostí pokračuje při řazení, práci s výběrem, změně aktivního
panelu i při obnovení stejné cesty. Samostatný identifikátor skenu a původní
nativní cesty oddělují jeho životnost od generace zobrazení. Recyklované číselné
ID nesmí připsat velikost jiné složce. Řazení používá přesný snapshot zobrazený
uživateli; nová ručně spuštěná úloha má vlastní oddělenou cache.

Průběžné překreslení velikostí se slučuje pro oba panely na250ms takt, tedy4×
za sekundu. Ostatní události a přímé ovládání reagují okamžitě. Nehotové součty
mají barvu a `≈`/`…`/`—`; vykřičník se nepoužívá. Dokončený částečný výsledek
si ponechá `≈`, ale běžnou barvou a váhou písma se odliší od aktivního výpočtu.
Řádky souborů ani složek nezobrazují hover tooltipy. Tooltipy ovladačů,
hlaviček, disků a systémových umístění zůstávají dostupné.
Průběžné výsledky ani dokončení výpočtu automaticky nemění pořadí řádků,
ani při zvoleném sloupci Size. Uživatel nové hotové velikosti uplatní ručním
řazením přes záhlaví nebo zkratku. Běžné načtení či obnovení seznamu používá
zvolené řazení.

Rychlá tlačítka disků a umístění se přesunou vedle nabídky disků, bez vlastního
horního řádku. V úzkém panelu se tento pás horizontálně posouvá; základní
navigační ovladače zůstanou přístupné. Podrobnosti jsou v
[aktuálním rozšíření návrhu](2026-11-04-selection-and-continuous-sizes-design.md).

## 29. Systémové ikony, úplná nabídka umístění a příkazová cesta

Rychlá navigační tlačítka a rozbalovací nabídka sdílejí jeden seznam všech
aktuálních disků a speciálních umístění. Domů, Plocha, Dokumenty a Stažené
soubory zůstávají samostatnými volbami i při shodné cílové cestě. Obě místa
poskytují tooltip s úplnou cestou; disky také s dostupným názvem svazku
a volným místem. Změny připojených disků nesmějí zanechat zastaralou volbu.
Identita a revize položky chrání před přesměrováním rozpracovaného kliknutí.

Tlačítka používají odpovídající systémové ikony, pokud je platforma poskytuje.
Windows a Linux mají oddělené adaptéry; načítání a dekódování provádí výhradně
backend s omezenou frontou a cache. Původní barevné pixely se nepřebarvují.
Kde vhodná systémová ikona chybí, zůstává srozumitelná vektorová ikona aplikace.
Ikony a popisky spodních funkčních tlačítek jsou společně vertikálně vystředěné
v obou hustotách ovládání, při zachování rozměrů tlačítek i poměru stran ikon.

Aktivní cesta příkazového řádku se zobrazuje celá, bez výpustky a bez pevného
omezení délky textu. Cesta a editor jsou v jediné vodorovné řadě se shellem,
Options a Run. Cesta má přirozenou šířku, pokud se vejde; u extrémních cest je
omezen pouze vodorovný posuvný výřez, aby editor zůstal použitelný.
Od revize 2026-11-07 je minimum přesně 320×200 logických pixelů pro obě hustoty
a všechny podporované velikosti písma; preferovaná velikost zůstává 1340×820.
Při nedostatku prostoru se zobrazí aktivní panel, lišta pro přepnutí druhého
panelu, cesta, soubory a příkazový řádek. Pomocné lišty se obnoví po zvětšení.
Funkční tlačítka lze vodorovně posouvat, zatímco Options a Jobs zůstávají viditelné.
Popisky všech Ctrl+F1–F12 vycházejí z aktuálního nastavení; nepřiřazená tlačítka
zobrazí klávesu a jsou zakázaná. Rodičovský řádek `..` není cílem souborových
operací; skutečný výběr dovoluje Copy/Delete i při zaměření rodiče.
Podrobnosti malého okna a jeho testů jsou v [ověření](../../verification/small-window.md).
Podrobnosti a ověřovací scénáře stanoví
[návrh systémových ovladačů a celé cesty](2026-11-04-native-controls-and-full-path-design.md).

Vkládání cest a jmen do příkazového editoru nepřidává uvozovky, escapování ani
oddělovací mezery. Jde o přesnou editaci textu, nikoli vytvoření chráněného
shellového argumentu. Pravidla ukládání nastavení, výběru barev/souborů,
responzivních dialogů a navazující optimalizace viewerů doplňuje
[ergonomie dialogů](../../developer/DIALOG_ERGONOMICS.md).

## 30. Hledání psaním, tooltipy a atribut komprese

Prosté psaní v seznamu souborů zahájí hledání začátku názvu. Po dodatečném
upřesnění uživatelem je hledání **vždy Unicode case-insensitive**, také na Linuxu.
Porovnává se původní název; nedochází k normalizaci identity ani odstranění
diakritiky. Hledání je doslovné, bez wildcardů a regulárních výrazů. Zvýrazní se
původní odpovídající prefix, při rozšíření case foldu až po konec původního
grafému. Nativní ne-Unicode jména se nesmějí hledat přes ztrátovou náhradu.

Šipky nahoru/dolů procházejí odpovídající soubory i složky v aktuálním řazení,
bez přetečení přes konec. Escape hledání opustí; Backspace odstraňuje celý
rozšířený grafém a prázdný dotaz režim ukončí. `..` zůstává navigací a nehledá se.
IME preedit je lokální; backend dostane až potvrzený text. Ctrl+Space patří IME,
Ctrl+P nadále vkládá cestu. Windows Shift+písmeno disku má přednost; Shift+číslo
pro uložené cesty je při hledání/kompozici dočasně potlačeno. Ostatní editory
zachovávají běžné chování.

Options nabízí filtrování při psaní: zobrazí jen shodné položky namísto skákání
v celém seznamu. Volba je ve striktních nastaveních verze4 jako
`quick_search_filter`, výchozí `false`; verze1–3 se migrují s vypnutým filtrem.
Předčasné místní změny přežijí pozdní načtení uložených nastavení. Dotaz je
samostatný pro každý panel, přežívá řazení a obnovu stejné složky; změna složky
nebo karty jej ruší. Prostor cesty slouží také pro kontextový dotaz bez trvalého
nového řádku UI.

Veškeré porovnávání a vytváření indexu probíhá na backendu nad neměnným úplným
snapshotem. Dva omezené, samostatné proudy přijímají poslední dotaz panelu;
starší práci kooperativně ruší a kontrolují generaci navigace, přesnou identitu
zdrojového Arc a revizi dotazu i po čekání na výstupní frontu. UI ověření opakuje.
Filtrování nemění kanonický obsah pro watchery a výpočet velikostí. Hromadný
výběr a vlastnosti používají skutečně zobrazenou projekci; skryté položky nejsou
akčními cíli. Klávesová aktivace během čekání nebo bez shod nesmí otevřít
zastaralou neodpovídající položku. Velká data se uvolňují na backendu.

Tooltipy mají menší odvozené písmo. Preferované umístění je vpravo dole od
kurzoru s mezerou přibližně14/18px; u hran se jednotlivé osy obrátí a bublina
zůstane ve viditelné ploše s8px okrajem. Velké bubliny se přizpůsobí dostupnému
prostoru, aby nezakryly kurzor a klikací místo. Zachová se úplný přístupný text,
průchod kliknutí a nativní životní cyklus i uvnitř popupů. Tooltipy na řádcích
souborů zůstávají odstraněné.

Soubor nebo složka s kompresí hlášenou filesystémem má malou značku u nativní
ikony. `EntryMetadata.compression` rozlišuje `Unknown`, `Uncompressed` a
`Compressed`; přípona archivu ani řídká alokace nejsou důkazem komprese.
Windows adaptér čte nativní atribut, Linux samostatně používá bezpečné `statx`
s kontrolou masky podpory. Metadata se načítají na backendu bez otevírání obsahu
a bez následování koncového odkazu. Zobrazená velikost zůstává logická.

Odkazy na složky a Windows junctiony mají rozpoznatelnou ikonu odkazu. Windows
načte systémový překryv na základní ikoně složky/souboru; ostatní platformy použijí
vhodnou systémovou ikonu, případně vlastní značku. Typ `DirectoryLink` zůstává
oddělený od skutečné složky: rekurzivní výpočet velikostí jej nesmí následovat.
Windows rozpozná adresářový odkaz z údajů enumerace bez dotazu na cíl; na Linuxu
se kvůli ikoně neprovádí implicitní průchod či síťový dotaz na cíl symlinku.
Načítání a skládání ikon probíhá na backendu. Překryv odkazu může současně
obsahovat označení komprese. Koncový odkaz se sleduje až při výslovné navigaci.

Podrobná rozhraní, prior art Salamanderu a ověřovací scénáře jsou v
[návrhu hledání, tooltipů a metadat](2026-11-05-panel-search-and-metadata-design.md).
[README](../../../README.md) obsahuje sestavení a spuštění Windows x64 a
Linuxu/Ubuntu/Mintu včetně `build.bat` a `build.sh`; macOS zůstává budoucí etapou.

## 31. Sdílení kódu a velikost generovaného Rustu

Opakované chování patří do sdílených funkcí a komponent s daty a callbacky.
Platformní implementace zůstávají oddělené. Tooltipy vlastní sdílené obaly;
vložené dítě v každé instanci by mohlo vynutit duplikaci komponenty. Generovaný
Rust se ručně nemění. Každé zmenšení musí zachovat Unicode, IME, focus,
přístupnost a vykreslení. Velikost Rust výpisu není velikost binárky ani RAM.

Předchozí optimalizace dosáhla 4,87 MB normalizovaných tokenů / 8,70 MB
formátovaného Rustu, o40,6 % méně než původní adaptér. Nejde o odhad nových
komponent ani tvrzení o optimu. Měření a další možnosti jsou v
[rozboru generování UI](../../verification/slint-code-generation-size.md).

## 32. Detailní navigace, historie a síťový provider

Kontrakty cest, historie posledních 100 unikátních příkazů, detailních lišt,
Space/Insert, Hot Paths, Options a výstupů pod `_build` jsou soustředěné v
[rozšíření panelů](2026-11-05-panel-navigation-details-design.md).
Novější [trvalé karty a nastavení v6](2026-11-05-tab-session-extension.md) přidávají
obnovu karet, režimů panelů a bezpečnou migraci nastavení při přejmenování produktu.
[Síťový filesystem](2026-11-05-network-filesystem-design.md) má samostatný
API-only plugin pro dostupná zařízení/služby/sdílení: Windows WNet/MPR a Linux
GIO/GVfs mají oddělené adaptéry. Panel nabízí Network, postupné výsledky, cache,
ruční cíle a výslovné předání share nativnímu filesystemu. Dostupnost nelze
zaručit pro zařízení blokující discovery; reálná SMB/FUSE interoperabilita ještě
vyžaduje řízenou kvalifikaci. macOS dostane vlastní adaptér později.

[Nové ovládání a layout](2026-11-05-panel-interaction-layout-design.md) definuje
Ctrl+Tab/Ctrl+U, splitter, karty, výběr, terminál, atributy, Light a F3/Alt+F3.
Poměr šířek panelů se ukládá mezi relacemi, nezávisle na velikosti okna a DPI.
Ukládá se po dokončení přetažení nebo klávesové úpravy; dvojklik uloží rozdělení
50:50. Dočasné zvětšení panelu Ctrl+F11 tento poměr nepřepisuje. Obnova proběhne
spolu s nastavením před prvním vykreslením; zápis zajišťuje asynchronní backend.

## 33. Nový název a navazující implementace

Produkt se jmenuje **Midnight Explorer**; release používá optimalizaci 3, fat LTO
a jednu codegen jednotku, s kontrolami přetečení a obnovou chyb workerů.
Podrobnosti jsou v [release konfiguraci](../../RELEASE_BUILD.md).
[Viewer rozšíření](2026-11-05-viewer-implementation-extension.md) nyní požaduje
tři samostatné pluginy, Escape/Space, velikost okna podle aplikace a externí editor F4.
Následuje ověřené dávkové copy/move, změna atributů/časů a nemodální hledání podle
[přijatého pořadí etap](../plans/2026-11-05-follow-up-stages.md).

## 34. Systémové nabídky, schránka a nastavitelné zkratky

[Rozšíření ovládání panelů](2026-11-06-panel-shell-design.md) přidává systémové
nabídky souborů a disků, Ctrl+C/Ctrl+V, ověřené kopírování s frontou, součet bajtů
vybraných souborů, volné místo, řazení ve všech režimech a oddělené akce Hot Paths.
Dvojklik zachovává identitu buňky i během překreslení. Ctrl+Enter vloží název
souboru a zaměří příkazový řádek. SHA-256 porovnává čtení zdroje s nezávislým
čtením zapsaného cíle; náhrada existujícího cíle a Move zatím zůstávají nepodporované.

[Nastavitelné klávesové zkratky](2026-11-06-configurable-shortcuts-design.md)
používají společný katalog akcí, kontexty, kontrolu kolizí, ukládání v Options
a obnovení výchozích hodnot. Standardní editace textu, IME a klávesy rezervované
operačním systémem zachovávají své nativní chování.

## 35. Řazení podle systémového locale

[Rozšíření řazení](2026-11-06-locale-sorting-design.md) používá nativní váhy názvů
a přípon včetně velikosti písmen a diakritiky. Shodné velikosti a časy se řadí
stejným pravidlem podle názvu. Locale se zachytí do restartu aplikace; Linux při
nedostupném locale použije C. Neplatné nativní kódování se nikdy nenahrazuje ani
nezkracuje: používá deterministické pořadí původních názvů. Klíče vznikají na
backendu, mají limit 4 MiB na klíč a zůstávají v paměti pro další řazení.
[Implementační plán](../plans/2026-11-06-locale-sorting.md) popisuje testy a
oddělené adaptéry Windows/Linux.

## 36. Osm témat a kompaktní příkazový řádek

[Rozšíření vzhledu](2026-11-06-theme-expansion-design.md) zachovává Blue, Dark
a Light a přidává Dracula, Nord, Gruvbox Dark, Catppuccin Mocha a Solarized Light.
Palety sdílejí sémantické barvy všech komponent; systémové ikony se nepřebarvují.
Nové palety mají testovaný kontrast textu nejméně 4,5:1 a zvýrazněných hran 3:1.
Volba se ukládá stabilním názvem. Editor příkazů, jeho cesta, historie i tlačítka
mají shodnou výšku se spodní funkční lištou, při zachování Desktop/Touch a fontu.

## 37. Porovnání, filtry, Ctrl lišta a editor metadat

[Návrh Commander nástrojů](2026-11-06-commander-tools-design.md) rozpracovává
nově zadané funkce podle zdrojů Salamanderu. Alt+nahoru/dolů v příkazovém editoru
cyklicky prochází uložených nejvýše 100 unikátních příkazů a nemění typ terminálu.
Enter příkaz spustí. Držení Ctrl přepne spodní lištu podle skutečně nastavených
vazeb Ctrl+F1 až Ctrl+F12. Uvolnění klávesy ani změna focusu nesmějí změnit akci
už stisknutého tlačítka; AltGr tuto vrstvu nezapíná.

Ctrl+F10 porovnává viditelné adresáře na backendu podle velikosti a změnového
času s volitelnou tolerancí (výchozí dvě sekundy). Označuje chybějící, větší nebo
novější soubory. Shoda metadat neznamená shodu obsahu. Rozšíření o porovnání
obsahu a rekurzi zůstává další etapou. Zrušený nebo zastaralý výsledek nemění
výběr. Ctrl+F12 nastavuje samostatný uložený filtr panelu: Masks/Regex ponechávají
složky viditelné, Expression vyhodnocuje typovaná metadata i u složek. Ctrl+F11
rozšíří aktivní panel na celou plochu obou panelů; druhý panel, jeho aktivační pruh
i dělič zcela skryje bez rezervovaného místa. Tab přepíná viditelný panel,
další Ctrl+F11 obnoví původní poměr šířek. Modely obou panelů zůstávají zachované.

Ctrl+F2 otevírá společný editor atributů, časů a velikosti písmen; Ctrl+F7 tentýž
editor se zapnutou změnou písmen. Výchozí stav ostatních polí je „beze změny“.
Náhled i provedení mají identitu operace a revizi návrhu, aby starší náhled nemohl
potvrdit jiné změny. Windows a Linux mají samostatné nativní implementace a své
schopnosti. Časy vyžadují explicitní časové pásmo; „aktuální čas“ zachytí backend
jednou pro celou dávku. Unicode převod zachovává původní nativní části názvu,
nikdy neodvozuje název ze ztrátového zobrazovacího řetězce. Rekurze nenásleduje
odkazy a zpracuje potomky před rodičem. Kolize nesmí přepsat jiný soubor.
Podrobný návrh stanoví limity zdrojů, částečné výsledky a testy nativních operací.

## 38. Přetahování souborů a zpřesnění vzhledu

[Návrh drag-and-drop](2026-11-06-file-drag-drop-design.md) přidává kopírování mezi
panely a předávání souborů jiným aplikacím. Příjem v aplikaci používá stejnou frontu
a nezávislé ověření SHA-256 jako schránka. Nativní transport má samostatné adaptéry
Windows a Linux/X11, omezené fronty a zachovává původní Unicode/nativní názvy.
Přetažení ven provádí cílová aplikace. Výchozí akce nepřesouvá ani nemaže zdroj.

Horní lišta má stejný odstup od disků jako disky od cesty. Dlouhá cesta zůstává
viditelná od konce, s výpustkou vlevo a možností otevřít celý editor. Viditelné
části zachovávají původní tokeny pro navigaci i kopírování. Všech osm témat má
výraznější rozlišení ploch, výběru a okrajů; kontrast se kontroluje společným testem.

## 39. Podporované systémy a minimální prostředky

Závazný přehled cílových verzí OS, architektur, knihoven a ověření obsahují
[systémové požadavky](../../SYSTEM_REQUIREMENTS.md). Pro verzi 0.1 je cílem
Windows 10 22H2 / Windows 11 x64 s MSVC, Ubuntu od 24.04 LTS a Mint 22.x x86-64.
macOS nemá implementovaný adaptér, proto zatím nemá podporovanou verzi ani
ověřené minimum RAM. Nová platforma musí projít vlastní kvalifikací.

Výslovné linuxové cíle zahrnují také udržované stabilní Fedory od verze 44 a openSUSE Leap 16.x
i Tumbleweed. Používají stejný linuxový adaptér; rozdíly patří do balíčkování,
závislostí a detekce schopností, nikoli do kopie implementace filesystemu pro každou
distribuci. README uvádí `dnf` a `zypper`. Nativní testy Fedory/openSUSE zatím nejsou
provedené; ověření musí pokrýt X11/Wayland, běžná bezpečnostní pravidla, Btrfs/ext4
a příslušné grafické ovladače. U Tumbleweed zaznamenáváme přesný snapshot. SUSE
Linux Enterprise potřebuje samostatně určenou a ověřenou verzi.

Pro běžný provoz plánujeme alespoň 4 GB celkové RAM (Ubuntu Desktop 26.04: 6 GB),
doporučeně 8 GB, a dostupnou rezervu 1 GiB pro aplikaci; pro velké adresáře
a souběžné úlohy alespoň 2 GiB. Jde o plánovací hodnoty, nikoli naměřenou dolní
mez nebo pevný limit procesu. Kopírování a checksumy jsou proudové a nesmějí
alokovat celý soubor. Testy na omezené RAM musí před vydáním ověřit velké výpisy,
ikony, hledání, více úloh a odezvu UI. Nároky pluginů se uvádějí samostatně.

Pro kompilaci doporučujeme 16 GB RAM, pro souběžné sestavení a fat LTO 32 GB.
Varianta 8 GB s jednou Cargo úlohou není zatím ověřené minimum. Vývojové cache
pro více platforem a profilů mohou zabrat přes 100 GiB; veškeré sestavené a dočasné
výstupy zůstávají pod `_build`. Distribuovaný Linux build používá nejstarší cílový
userspace Ubuntu 24.04 / glibc 2.39. Bezpečné nativní operace mají navíc konkrétní
požadavky na kernel a schopnosti souborového systému; neúspěch nikdy nevede
k vypnutí kontrol identity nebo ověření kopie.

Ověření Unicode musí kontrolovat skutečný renderer a viditelné glyphy. Na virtuálním
Linux displeji může Slint bez vhodného GPU přejít na softwarový renderer, který
nezobrazuje bitmapové emoji. QA může pro ověření WGPU použít `SLINT_WGPU_CPU=1`
a testovací fonty; takový výsledek není kvalifikací fyzického GPU ani výkonu.

## 40. Ikona aplikace

Ikonou Midnight Exploreru je měsíc se světlou stříbrnou a tmavou modrošedou
polovinou na průhledném pozadí. Okno používá vložený PNG zdroj; Windows EXE
obsahuje ICO s velikostmi 16 až 256 px pro Průzkumník, titulkový pruh a hlavní
panel. Systémové ikony souborů a disků tím nejsou nahrazeny. Zdrojový obrázek,
exporty a popis jejich původu jsou v `assets/branding`; běžná kompilace nepotřebuje
obrázkovou službu ani nástroje pro převod grafiky. Windows resource compiler běží
pouze při sestavení a jeho výstup zůstává v Cargo `OUT_DIR` pod `_build`.
Wayland používá shodný XDG identifikátor aplikace, desktopový soubor a nainstalovanou
PNG ikonu; samotná vlastnost ikony okna tam nestačí. Postup instalace je v README.

## 41. Přehlednější nastavení, klávesové zkratky a úlohy

Nastavení používá větší okno omezené rozměry hlavního okna, s tématy vlevo,
posuvným obsahem a pevnou spodní řadou akcí. Rozměry respektují desktop/touch
profil a zvolené písmo. Kategorie lze přepínat myší i klávesnicí; změna kategorie
šipkami musí přesunout také fokus.

Editor zkratek vychází z dodané ukázky Visual Studia: vyhledávání podle názvu
příkazu nebo ID, virtualizovaný seznam příkazů, výběr existující zkratky a záznam
nové skutečným stiskem kláves. Assign přidává alias, Remove odstraní pouze zvolený
alias. Před přiřazením se zobrazí kontext a aktuální vlastníci zkratky; konflikt
v překrývajícím se kontextu neprovede částečnou změnu. Záznam kláves nikdy
nespouští příkazy aplikace. Tab/Escape zaznamená klávesu a opustí záznamové pole.
Obnova celého výchozího mapování vyžaduje potvrzení v editoru. Kontext určuje
příkaz a pravidla pro editaci textu; nabízíme pouze skutečně implementované
kontexty a mapování. Podrobnosti: [návrh editoru](2026-11-07-keyboard-options-design.md).

Jobs používá virtualizovaný seznam operací a samostatný detail vybrané úlohy.
Výběr je určen stabilním ID, takže jej nezmění průběžná aktualizace ani nová
úloha. Zobrazuje ikonu, stav, fázi, úplné cesty a známé počty/bajty. Kopie bez
známého celkového objemu neukazuje odhadnutá procenta. Skutečná chyba má výraznou
ikonu; čekání na rozhodnutí není chyba. Cancel, Resolve, Copy details a Close
jsou ve spodní řadě; kopírování detailu používá backend schránky. Zavření okna
ponechá úlohy běžet a stisk akce sám okno nezavírá.

Nabídka disků obsahuje navíc Pictures, Videos, Music, uživatelský TEMP a dostupné
osobní/pracovní účty OneDrive. Windows cesty pocházejí z Known Folders a registrací
účtů, takže respektují přesměrování. Linux používá XDG user directories a své
dočasné umístění. Neexistující složky nevytváříme; všechny dotazy a ikony zajišťuje
backend příslušné platformy. Podrobnosti: [umístění a Jobs](2026-11-07-locations-and-jobs-design.md).

Pokud jsou ve výběru složky s dokončeným bezchybným měřením, stavový řádek ukáže
i jejich známý součet. Částečné nebo chybějící výsledky musí zůstat rozeznatelné
od konečné velikosti. Součty zpracovává backend pro zachycený výběr a verzi výpisu;
UI nesmí procházet velký výběr při každém překreslení.

## 42. Samostatné archivní a mediální pluginy

Ve stejném Git repozitáři zůstávají nezávislé Cargo projekty: veřejné protokoly
`filesystem-api`, `thumbnail-api` a `worker-api` a samostatné implementace archivů,
obrázků a videí. Pluginy mají vlastní testy, závisí pouze na verzovaném veřejném
API a nemění zdrojové soubory hostitele. Nasazují se vedle programu do
`filesystems/` a `thumbnailers/`; chybějící plugin nezpřístupňuje danou funkci.
Dosavadní vydané viewer API/SDK 0.1.0 se tím nemění.

Archivní provider nabízí čtení ZIP, 7z, GZIP, TAR a TAR.GZ, navigaci, hledání,
řazení a F3 nad zachyceným obsahem. Identitu člena netvoří lokální cesta. Záložky
uchovávají typovaný popis provideru, zdrojového archivu a nezměněné komponenty;
po změně nebo ztrátě zdroje zůstanou výslovně nedostupné. První implementace má
rozpočty 64 MiB pro archiv/člena a 128 MiB pro dekódovaný součet. Neprovádí zápis
do archivů, vnořené mounty, dešifrování ani spuštění člena externí aplikací. F5
vykopírování do nativního cíle doplňuje navazující etapa z 2026-11-07.
Tyto rozpočty neomezují délku nativních cest. Přesné smlouvy a další limity uvádí
[filesystem API](../../developer/FILESYSTEM_API.md).

### Rozpoznávání a volitelné asociace archivů

Hostitel sdílí jednu tabulku 67 přípon (51 ZIP kontejnerů a šestnáct přípon ostatních
dekodérů). Seznam a zdroje jsou v [archivních asociacích](../../ARCHIVE_FORMATS.md).
Options → Archives nabízí každou příponu se zaškrtávátkem a ukládá jednotlivé
uživatelské odchylky od výchozího stavu. ZIP a vývojářské balíčky WHL, EGG, WAR,
EAR, AAR, AAB, NUPKG a SNUPKG jsou zapnuté. Dokumenty Office/OpenDocument, knihy,
komiksy, mapy, obrazové projekty, spustitelné JAR/PYZ/PYZW a APK/VSIX/XPI zůstávají
v seznamu vypnuté, aby Enter běžně otevřel systémovou aplikaci. Uživatel je může
jednotlivě zapnout. Nejde o změnu asociací operačního systému.

Backend při výpisu připraví kandidátní typ bez čtení obsahu. Značka vstupní šipky
u systémové ikony se vykreslí ve všech třech režimech pouze pro nativní soubor se
zapnutou asociací a skutečně dostupným providerem daného formátu. Přítomnost
provideru jiného formátu nestačí. Pozdní dokončení discovery či změna nastavení
aktualizuje značky; metadata komprese filesystemu a systémové link overlay zůstávají
samostatné. Ikona znamená dostupný handler, nikoli předem ověřený obsah souboru.
Plugin validuje obsah až při otevření na backendu. Vnořené archivy ani adresáře
s archivní příponou nedostávají značku nepodporovaného vstupu.

Provider umí F3 čtení a F5 vykopírování do nativního cíle přes stávající frontu,
bezpečnou přípravu cíle, konfliktní dialog, průběh a SHA-256 kontrolu nezávislým
čtením cíle před zveřejněním. Zápis/přesun uvnitř archivů zůstává budoucí etapou.

Obrazové a video náhledy vznikají v omezených pomocných procesech, na Windows
v AppContaineru s Job Objectem a na Linuxu se seccomp a limity prostředků.
Hostitel zprostředkuje pouze čtení zachyceného obsahu; dekodér nemá přístup
k libovolným souborům ani síti. Nejvýše dvě dekódovací úlohy sdílejí omezenou
frontu a rozpočet paměti. Přístup ke zdrojovým datům používá společný rozpočet
s měřením složek a kopírováním: souběh dovolují pouze potvrzená lokální SSD,
HDD/síť/neznámé úložiště mají sériový přístup po omezených blocích.

Každý úspěšný snímek se nejprve bezpečně uloží do soukromé diskové cache.
Video normálně ukazuje jeden snímek; všechna viditelná označená videa postupně
střídají nejvýše šest snímků, s předností souboru pod myší. Verze zdroje, změna
panelu i vymazání cache musí zneplatnit staré výsledky. Nastavení Previews umožní
náhledy vypnout, výslovně povolit síťová umístění a asynchronně vymazat cache.
Podrobnosti: [náhledy](2026-11-05-panel-views-thumbnails-design.md) a
[worker SDK](../../developer/WORKER_SDK.md).

Seznam všech knihoven a skutečně připnutých licencí je v
[licenčním přehledu](../../THIRD_PARTY.md). Distribuce přikládá příslušné copyright
a licenční texty; volitelný FFmpeg také odpovídající zdroj a konfiguraci sestavení.

## 2026-11-07: prohlížeče, barvy a navigace

[Aktuální rozšíření](2026-11-07-viewer-images-file-colors-design.md) doplňuje
zalamování Text/Source pomocí Ctrl+W, ověřené automatické binární zobrazení se
sloupci offset/hex/znaky a hledáním a samostatný obrazový plugin. Obrazy používají
stejné knihovny `image 0.25.10` a `moxcms 0.8.1` jako náhledy. Izolovaný proces
drží dekódovaný originál; host zprostředkuje čtení a dostává pouze omezený výřez.
Fit, 1:1, přiblížení kolem kurzoru a tažení fungují bez I/O na UI vlákně.
Public API/SDK, testy a plugin zůstávají samostatnými Cargo projekty; viz
[API obrazového vieweru](../../developer/IMAGE_VIEWER_API.md).

Options → Colors obsahuje pořadí pravidel masek/regexu, jednu základní barvu
písma a volitelné pozadí. Barevný vzorek otevírá samostatný přesouvatelný a
zvětšitelný popup nad Options; HSV/RGB ovladače a paleta se posouvají, zatímco
Use color a Cancel zůstávají dostupné i při 320 × 200. Escape a křížek zruší pouze
picker; výběr se ukládá až přes OK v Options. Podrobnosti jsou v
[návrhu transakce nastavení](2026-11-08-options-transaction.md).
Stav focus/selection se odvozuje automaticky s kontrolou
kontrastu; čtyři stavy jsou vidět v náhledu. Vyhodnocení probíhá na backendu proti
neměnnému snapshotu. Ctrl+F12 vedle masek a regexu nabízí Expression nad jménem,
velikostí běžného souboru, časem změny, druhem a hlášenými nativními atributy.
Výrazy používají `not`/`and`/`or`, typované hodnoty a výslovné UTC nebo číselný
časový posun. Chybějící metadata zůstávají neznámá i po negaci; jejich zahrnutí
určují `missing(...)`/`present(...)`. Neznámé kódování archivního jména se nenahrazuje
zobrazeným popiskem. Linux dot-jméno nepředstírá nativní příznak hidden.
Backend kontroluje 4096 UTF-8 bajtů, 512 tokenů, 256 uzlů, zanoření 32 a nejvýše 16
jmenných vzorů; neprovádí I/O, skripty ani čtení obsahu. Chybný či zastaralý návrh
nezmění platný filtr; dialog zachová text a ukáže řádek/sloupec chyby. Verze
nastavení 18 zachovává dřívější příkazy i volby kontrolních součtů. Kontext předků v inline stromu
čeká na TREE-01. [Syntaxe a limity](../../developer/METADATA_FILTERS.md),
[ověření Windows/Linux](../../verification/2026-11-07-metadata-expression-filters.md).
Carbon Green je deváté téma,
přidané bez změny identit předchozích témat.

Nabídka disků má neutrální tlačítko, využívá dostupnou výšku okna a zachovává
označení aktuálního disku v nabídce i mezi tlačítky. Oblíbené cesty jsou pouze
v nabídkách s čísly 1–0. Tlačítko Hot Paths je v pravé části horní lišty každého
panelu vedle voleb zobrazení a řazení. OneDrive má jeden primární zástupce na liště, zatímco
různé účty mají rozlišené položky v nabídce; stejné fyzické cíle se neduplikují.
Ikony zvláštních složek jsou větší v nezměněných tlačítkách. Archivní cesty mají
nativní oddělovače bez vložených mezer. Generickou ikonu listu u podporovaného
archivu nahrazuje vlastní SVG; konkrétní systémové ikony mají přednost.
[Původ prostředků](../../ASSET_PROVENANCE.md) je zdokumentovaný samostatně.

## 2026-11-07: rychlejší výpis, extrakce, PDF a porovnávání

[Navazující návrh](2026-11-07-archive-copy-pdf-comparison-design.md) specifikuje
nové hranice pluginů a kopírování. Windows výpis využívá metadata vrácená již
enumerací; další dotazy zůstávají pro providery, které metadata nedodaly. Výpis
nikdy nenahrazuje ověření identity při pozdější operaci. Místní měření je v
[ověření výkonu](../../verification/2026-11-07-listing-performance.md).

ZIP načítá seznam bez rozbalení všech souborů. Člen se dekóduje a ověří až při
čtení. Přibyly XZ, Bzip2 a Zstandard a odpovídající TAR varianty; GZIP/TAR/solid
7z mají nadále omezení sekvenčního zpracování. F5 zachytí provider, položky,
výběr a cílovou složku. Backend odmítne únik cestou, odkazy, cykly a změněné
zdroje; nativní cíl vzniká přes dočasný soubor a nezávislou SHA-256 kontrolu.
Existující cíle se nepřepisují bez implementované bezpečné politiky konfliktů.

Samostatný PDF plugin používá bezplatný open-source Hayro. Veřejné raster API
0.2 přidává seznam stránek a vykreslení konkrétní stránky při zachování
obrazového protokolu. Stránky přepínají tlačítka nebo Ctrl+PageUp/Down; zoom,
posuv, Escape a Space navazují na stávající viewer. PDF nespouští skripty ani
externí odkazy; nekompatibilní dokument skončí s chybou, nikoli tichým úspěchem.

Textový porovnávač má [vlastní výběr neproporcionálního písma](2026-11-08-comparator-font.md)
v liště. Výchozí je dostupné Consolas, jinak systémové monospace; uživatelská
volba se ukládá samostatně a nemění písmo aplikace ani viewerů.

Ctrl+Shift+C porovná dva vybrané nativní soubory v aktivním panelu. Jinak použije
jeho vybraný/zaměřený soubor a výslovný výběr v protějším panelu, nebo jeho jediný
viditelný protějšek se stejným nativním názvem. Nelze-li dvojici určit, nabídne
**File A / File B**, každé s **Browse…**, a **Compare / Cancel**. Známý soubor
zůstane předvyplněný na své straně; chybnou cestu lze opravit ve stejném okně.
Formulář je dostupný i bez cílové položky v panelu. Přijímá absolutní cesty bez
shellové interpretace; systémový picker i validace běží na backendu.
Escape/křížek ruší okno i případný picker. Text používá `similar` s limitem vstupu a času; obrázky
sdílejí izolovaný dekodér se zobrazením Side-by-Side, Swipe a Onion Skin.
Výpočet i rasterizace běží mimo UI. Nejednoznačný výběr vyžádá ruční dvojici. PDF se
nevydává za jednosnímkový obrázek k porovnání celého dokumentu.

Výchozí barevná pravidla rozlišují zdrojový kód, spustitelné soubory, obrázky,
videa a dokumenty. Uživatelská pravidla zůstávají zachována a výslovné odstranění
všech pravidel se ukládá. Ikony na liště disků mají 20 px pro myš a 26 px pro
dotyk, se stejnou výškou a šířkou tlačítek. [Vlastní příkazy](2026-11-07-user-commands-design.md)
jsou zatím návrh podle Salamander User Menu: lišta nad příkazovým řádkem,
uživatelské ikony/popisky, pracovní složka a přímý vstup do příslušného nastavení.

## 2026-11-07: okna, kompaktní rozhraní a ochrana navigačních položek

Minimální velikost hlavního okna je **320 × 200 logických pixelů**, nezávisle na
hustotě a zvoleném písmu. Při nedostatku místa se zobrazí aktivní panel a úzký
přepínač druhého panelu; příkazový řádek, Options a Jobs zůstávají dostupné.
Funkční tlačítka lze vodorovně posouvat. Po zvětšení se vrátí běžné dva panely.
Nepřiřazené Ctrl+F1–F12 mají viditelný popisek a jsou zakázané. Kliknutí zachytí
akci i panel při skutečném stisku, ještě před případným odložením události ve
Slint Flickable; uvolnění Ctrl během kliknutí nesmí změnit význam operace.

Položka `..` slouží pouze navigaci. Kopírování, mazání, přejmenování, editace
atributů a drag-and-drop ji nikdy nepoužijí jako zdroj. Skutečně označené položky
lze kopírovat nebo smazat i s kurzorem na `..`; přejmenování a viewer vyžadují
skutečnou zaměřenou položku. Vytváření souboru/složky a vložení do aktuálního
umístění zůstávají dostupné. UI používá identity položek a backend vlastní
validaci; text názvu souboru není bezpečnostní identita.

Dialogy mají tažitelné záhlaví a polohu omezenou rozměry okna. Zmenšení nesmí
znepřístupnit zavření ani ztratit rozepsané hodnoty. Editor atributů, času a
velikosti písmen má vlastní SVG ikonu dokumentu s hodinami; nadbytečný spodní
seznam náhledu souborů byl odstraněn. Validace požadovaných změn a následné
potvrzení přes připravené identity zůstávají na backendu.

Výběr atributů používá pět kompaktních třístavových ovladačů s viditelným stavem
Ponechat/Nastavit/Zrušit místo patnácti tlačítek. Podle dostupné šířky a fontu
jsou atributy ve dvou nebo jednom sloupci; výchozí dialog míří na 640 × 440 px.
Přechod na kontrolu změn, zpět a mezi tématy nejprve bezpečně uzavře tooltipy
odstraňovaných ovladačů. Kontrola beze změn je platná, ale nikdy nepovolí zápis.
Podrobnosti: [kompaktní editor atributů](2026-11-08-compact-attributes.md).

Nastavení v15 ukládá fyzickou pozici vnějšího rámu, fyzickou velikost klientské
oblasti, normální rozměry před maximalizací a identifikaci monitoru. Změny se
ukládají asynchronně po 600 ms klidu, nejpozději po 2 s souvislé změny a při
ukončení. Dokud nejsou načtena uložená nastavení, nové výchozí rozměry je nesmějí
přepsat. Odpojený monitor znamená přesun do pracovní oblasti primárního monitoru.
Viewery a porovnávač přebírají fyzické rozměry a pozici hlavního okna; Escape je
vždy zavře i při změně uživatelského mapování. Omezení Waylandu a ověření jsou v
[dokumentu geometrie oken](../../verification/window-geometry.md).

Výběr obdélníkem začíná v prázdném místě panelu. Details, Brief a Thumbnails
používají dávky nesouvislých rozsahů vůči zachycené revizi seznamu a výběru;
podporují Ctrl/Shift, archivy a posun při přiblížení k okraji. Změna řazení,
filtru nebo složky zneplatní staré gesto. Tažení skutečné položky zachovává
souborový drag-and-drop. Aktuální implementaci a ověření popisuje
[výběrový kontrakt](../../developer/SELECTION.md).

### Spuštění pluginů ze sdíleného Windows buildu

Build helper vytvoří soukromou instalaci a registraci svázanou se SHA-256 přesného
host EXE. Spuštění `_build/release/me.exe` pak může použít bezpečné pluginy této
instalace, i když samotný checkout leží na sdíleném disku. Stejný backendový
resolver používají viewery, náhledy, archivy i obrazový porovnávač. Registrace ani
její cíl nesmějí být přepsatelné jiným účtem nebo vést přes junction. Kontroly
pluginů zůstávají aktivní a při startu se žádný plugin automaticky nekopíruje.
Soukromé přenosné instalace nadále používají sousední složky. Viz
[návrh](2026-11-07-trusted-runtime-resolution.md) a
[vývojářský kontrakt](../../developer/RUNTIME_DISCOVERY.md).

### Samostatné hledání souborů

Alt+F7 nebo lupa v panelu otevře samostatné posuvné okno hledání. Nejvýše čtyři
okna uchovávají nezávislé kořeny, dotaz a výsledky; panely lze dál používat.
Masky podporují více vzorů i vyloučení, výchozí `*.*` zahrnuje i názvy bez přípony.
Rekurze je zapnutá, rozlišování velikosti písmen volitelné. Obsah se hledá jako
doslovný text v UTF-8, případně s BOM, průběžně na backendu bez načtení celého
souboru do paměti. Nepodporovaná kódování a omezení jsou viditelná v souhrnu.

Výsledky přicházejí v omezených dávkách; Stop ponechá dosavadní výsledky a Escape
zavře konkrétní hledání. Enter, dvojklik nebo Go to file požádá backend o kontrolu
identity a revize výsledku a potom otevře nadřazenou složku v původním panelu.
Soubor se touto akcí nespouští. Nativní cesty se zachovávají odděleně od textu UI.
Otevření, editace či mazání z výsledků vyžaduje samostatný bezpečný backendový
kontrakt. Rozsah a limity: [Find API](../../developer/FIND.md).

### Priorita názvu ve stavovém řádku a čitelnost vieweru

Stavový řádek panelu přidělí nejprve prostor názvu souboru nebo souhrnu výběru,
nejvýše do šířky panelu. Ostatní metadata a volné místo použijí zbývající prostor;
název nemá pevný limit 220 px. Oříznutí nemění úplnou kopírovanou hodnotu.
Atributy Windows se zobrazují jednotlivými znaky (`A` archive, `C` compressed,
`R` read-only atd.), s vysvětlením přítomných příznaků v tooltipu. Neznámý bit je
`?`, nikoli hexadecimální hodnota. Unixová oprávnění zůstávají symbolická a oktalová.

Viewer rozpozná i tisknutelný UTF-8 text s nulovými oddělovači, například soubory
argumentů `.nul`; nulová výplň a jiné binární znaky nadále volí binární režim.
Zvýraznění syntaxe používá syté barvy upravené proti skutečnému pozadí na kontrast
nejméně 4,5:1, bez plošného míchání s běžným textem. Výpočet kontrastu sdílí
s panely a paletu počítá jednou na stránku. Při scrollování již cachované glyfy
nevyžadují opakované vytváření fontového scaleru. Okamžitá odezva při stránkování
a u extrémně dlouhých řádků zůstává samostatným cílem výkonnostního ověření.
Viz [testy a měření](../../verification/2026-11-07-status-and-viewer-readability.md).

### Rozbalovací větve v panelu — backendový mezikrok 8. listopadu

Každá větev má vlastní neměnnou identitu výskytu a zachycený kontext filesystemu.
Pozice řádku ani zobrazený název nesmějí určovat cíl souborové operace. Backend
odděluje požadavek na načtení od instalace výsledku: sbalení větve, výměna mountu
nebo navigace zneplatní opožděný výsledek. Rušení neuvolní kapacitu workeru,
dokud skutečné volání neskončí. Stav neznámý, načítání, prázdná větev a chyba
zůstává odlišný; `+` značí známé potomky, `-` rozbalení.

Implementovaný model podporuje nezávislé větve a připojení explicitně otevřeného
archivu bez převodu člena archivu na falešnou nativní cestu. Zachovává soubory
v kořenovém seznamu a z podsložek vkládá pouze složky. Limity modelu nesmějí
omezit běžné zobrazení kořenového seznamu ani délku filesystemové cesty.

Panely nyní model používají pro nativní a archivní větve, výběr, navigaci,
řazení sourozenců, prefixové hledání a filtry s kontextem předků. Výpočet velikostí
také zachycuje typované cíle: označené složky, bez výběru všechny zobrazené složky
včetně potomků rozbalených větví (i mimo aktuální výřez při scrollování),
a při Space pouze původní složku pod kurzorem. Výsledky se publikují nejvýše
čtyřikrát za sekundu pro panel a samy nepřerovnávají řádky. Změna výběru, řazení
nebo filtru přijatý výpočet neruší; navigace ano. Rušený worker drží kapacitu až
do skutečného návratu. Archivní výpočet čte metadata bez rozbalování obsahu.

Ctrl+Shift+S zachytí rozsah při přijetí požadavku. Nezahrnuje skryté či sbalené
potomky, pomocné předky filtru ani junctiony. Změna velikosti okna nebo scroll
rozsah neovlivní. Při existujícím výběru stále počítá pouze označené složky;
samotný výběr souborů nespustí výpočet všech složek. Toto rozšíření neovlivňuje
Ctrl+A, které nadále označuje pouze položky nejvyšší úrovně panelu.

Další konzumenti bezpečných cílů operací a produkční připojování vnořených
kontejnerů zůstávají nedokončené. Nativní změnové události a ruční refresh nyní
zachovávají přijatý strom; sledování archivních větví zůstává samostatnou etapou.
Kontrakt a přesný stav ověření jsou v
[API větví](../../developer/INLINE_TREES.md) a
[produkčním napojení](../../developer/TREE_PANEL.md), včetně
[výpočtu velikostí](../../verification/2026-11-10-tree-folder-sizes.md).

### Zjednodušení ovládání — 8. listopadu

Společná horní lišta ponechává akce pro aktivní panel. Volba zobrazení a řazení
se vrací na diskovou lištu každého panelu; ve velmi malém okně sdílí řádek s cestou.
Otevřený výběr zůstává svázaný se svým panelem a při změně jeho adresáře se zruší.

Options je ozubené kolo na pravém konci horní lišty. Vpravo vedle něj je původní
ikona měsíce, která otevírá About s názvem a verzí aplikace. Obě tlačítka zůstávají
pevná i při posouvání ostatních nástrojů. Spodní lišta obsahuje F1–F12 a Jobs.

Dialog filtru nemá samostatný checkbox zapnutí. OK ověří a použije výraz,
Cancel zahodí rozpracovanou změnu a Show all vypne dosavadní filtr. Stejnou
okamžitou akci má křížek v indikátoru filtru. Poslední výraz a režim zůstávají
zapamatované; opožděné ověření nesmí vypnutý filtr obnovit.

V nastavení archivů mají vestavěné přípony checkboxy, vlastní přípony křížek
pro odstranění. Malé + na konci seznamu otevře vstup pro další příponu.
Vlastní přípony i + jsou zarovnané vlevo s vestavěným seznamem; nesmějí se
automaticky centrovat podle šířky formuláře. Dlouhé přípony se zalamují.
Úpravy zůstávají součástí společného potvrzení OK/Cancel v Options.

Ikony příkazů používají původní SVG kresby s omezenými barevnými akcenty a
variantami pro světlé a tmavé pozadí. Nativní systémové ikony si zachovávají
barvy. Schválená ikona komprimovaného archivu se nemění. Moderní návrhové
reference a kroky jsou v [plánu úprav](../plans/2026-11-08-ui-polish.md).

### Přehlednější kontrolní součty — 8. listopadu

Modeless okno Checksums upřednostňuje tabulku výsledků. Má stránky Results,
Options, Save manifest, Details a Verify file; v úzkém okně je přepíná jeden rozbalovací
seznam. Nastavení algoritmů a rekurze nezabírá místo nad výsledky a během práce
je uzamčené. Tabulka zobrazuje zvolený algoritmus; název, počet bytů a stav jsou
před hashem, takže chyby nejsou schované za několika dlouhými kontrolními součty.
Šířka hashe se měří podle skutečného monospace fontu a celé hodnoty lze posouvat.

Copy pojmenovává zvolený algoritmus. Ctrl+C kopíruje jeho hodnotu, Ctrl+Shift+C
celý řádek; místní nabídka zachovává všechny vypočtené algoritmy. Save manifest
otevře samostatný formulář, Save jej odešle až s cílem a případným výslovným
souhlasem s částečnými výsledky. Nový výpočet souhlas zruší, změna stránky ne.
Close je stále dostupné, chyby mají ikonu a křížek. Okno zůstává měnitelné od
320×200 a vyšší fonty používají posouvání obsahu. Podrobnosti:
[návrh UI](2026-11-08-checksum-polish-design.md).

Kontrolní součty lze ukládat i ověřovat ve všech pěti formátech checksum pluginu
Salamanderu: SFV/CRC32, MD5, SHA-1, SHA-256 a SHA-512. Save manifest nabízí formát
s odpovídající příponou a systémový dialog pro cíl. Verify file umožňuje vybrat
manifest; explicitní Verify spustí kontrolu na backendu. Vedle GNU seznamů načítá
také BSD zápis, komentáře a samotný hash v souboru s odpovídající příponou.
Výsledky odlišují shodu, rozdíl, chybějící soubor a chybu čtení. Zrušení zachová
hotové výsledky. Parser a nativní čtecí handly brání úniku mimo složku manifestu,
průchodu odkazy i záměně načítaného souboru; nevalidní Unicode se tiše nepřevádí.
Přesné formáty, limity a rozhraní: [checksum API](../../developer/CHECKSUMS.md).

### Průběžné odznačování kopírování a přesunu (8. listopadu 2026)

Každá úspěšně dokončená vybraná položka se průběžně odznačí. Kopie musí být
ověřená SHA-256 čtením cíle a zveřejněná; přesun musí navíc dokončit odstranění
zdroje. Složka se odznačí až po dokončení celého podstromu. Přeskočené, chybné
a nedokončené položky po zrušení zůstanou označené. Kurzor se neposouvá.
Backend předává potvrzení jednotlivých kořenových položek nezávisle na čítači
průběhu; UI posílá malé dávky změn výběru zpět výběrovému workeru. Obnova seznamu
zachovává identitu podle neztrátové cesty, archivy podle mount/entry ID. Změna
složky či karty odděluje nový výběr od opožděných potvrzení původní úlohy.
Podrobnosti a testy: [transfer selection](../../verification/2026-11-08-transfer-selection.md).

### První zobrazení a obnova nedostupných cest (8. listopadu 2026)

Hlavní okno zůstává skryté, dokud backend nedokončí načtení nastavení a UI
neaplikuje uložený motiv, písmo a hustotu. Nativní okno se připraví skryté,
obnoví uložený monitor, polohu, velikost a stav a teprve potom se zobrazí.
Odpojený monitor se nahrazuje primárním podle existujícího pravidla dostupné
pracovní plochy. Chyba nastavení otevře aplikaci s výchozími hodnotami a chybou;
nepovoluje jejich automatické uložení přes nečitelné nastavení.

Nelze-li otevřít nativní cestu, backend postupně zkouší její skutečné nadřazené
adresy až ke kořeni. Pokud není dostupný ani ten, použije `/` na Linuxu nebo
kořen disku obsahujícího instalaci Windows. Neúspěšný poslední kořen skončí
chybou, nikoli další smyčkou. Každý další pokus má novou generaci navigace;
novější ruční navigace se opožděným výsledkem nepřepíše. Neúspěšné cesty se
nepřidávají do historie. Poskytovatelské/archivní identity se tiše nepřevádějí
na nativní cesty. Veškeré čtení a dotazy na kořen OS zůstávají v backendu.

Záhlaví dialogů mají ikony podle významu a společné čtvercové tlačítko zavření
s geometricky centrovaným vektorovým křížkem. Hot Paths má samostatné,
nepřekrývající se oblasti hvězdy a rozbalovací šipky. Save manifest otevírá
přímo systémový dialog pro výběr cílového souboru; ruční editace cesty není
mezistupněm. Pravidla explicitního potvrzení neúplných výsledků, nového souboru
bez tichého přepsání a ověření zápisu zůstávají zachována.

Ověření: [atributy a ovládací prvky](../../verification/2026-11-08-metadata-review.md),
[první snímek](../../verification/2026-11-08-startup-first-frame.md),
[obnova cesty](../../verification/2026-11-08-navigation-fallback.md).

### Chyby přenosu a zavírání průběhu (8. listopadu 2026)

Copy/Move při obnovitelné I/O chybě pozastaví úlohu a podle skutečných možností
backendu nabídne Retry, Skip, Skip all this error type a Cancel. Platí i pro
nedostupný cíl, plný disk a selhání čtení, zápisu či synchronizace. Čekání neblokuje
UI ani I/O worker; zachovává pořadí mutací ve frontě. Každé rozhodnutí patří
konkrétní úloze a verzi a nemůže nahradit souhlas s přepsáním souboru.

Retry zachovává identitu otevřeného zdroje a cíle i rozpracovaný blok. Cíl vždy
projde úplným SHA-256 readbackem. Chyba odstranění zdroje při Move neopakuje kopii:
před novým pokusem o odstranění znovu ověří již uložený cíl a držený zdroj.
Chyba enumerace složky se nesmí považovat za konec seznamu. Přeskočení zachová
zdroj i potřebné nadřazené složky. Změna identity, neshoda kontrolního součtu
nebo nejistý výsledek publikace nepovoluje nebezpečné opakování.

Po úspěchu se zavře pouze automaticky otevřený průběh. Ručně otevřené Jobs zůstane
otevřené; záznamy se uzavřením okna nemažou. Chyby a zrušené či přeskočené úlohy
zůstávají viditelné. Escape/Decide later odloží rozhodnutí do Jobs → Resolve.
Podrobnosti, omezení a porovnání se Salamanderem:
[návrh obnovy přenosu](2026-11-08-transfer-errors-design.md),
[API přenosů](../../developer/TRANSFERS.md).

### Klávesové procházení rozdílů a plynulé porovnání obrázků (8. listopadu 2026)

Komparátor má nastavitelné akce pro další, předchozí, první a poslední textový
rozdíl. F7/Shift+F7 zůstává zachované; výchozí aliasy Alt+Down/Up,
Alt+Right/Left, N/P a Alt+Home/End vycházejí z komparátoru Salamanderu.
Skok zarovná změněný řádek v obou pohledech, na krajích nepřetáčí a rychlá
opakování kláves neztrácí slučováním vstupů. Přenastavení nahradí všechny
výchozí aliasy; editační pole si ponechávají své klávesy. Přehled:
[klávesové zkratky](../../KEYBOARD_SHORTCUTS.md).

Režimy Swipe a Onion Skin skládá renderer ze dvou uchovaných obrazů pomocí
ořezu a průhlednosti. Pohyb dělicí čáry nebo změna opacity již nevyvolává
přepočet a přenos celého RGBA snímku. Dekódování, vzorkování a příprava
průhledného pozadí zůstávají v backendu a izolovaných image workerech.
Cache uchovává rozložení vedle sebe a společné překrytí; první použití
rozložení stále vyžaduje vzorkování, další přepnutí je znovu využijí.
Změna velikosti, zoom nebo posun cache zneplatní. Lichá šířka dává pravé
polovině jeden fyzický pixel navíc při stejném měřítku a počátku.

Posuvník označuje Divider nebo Right opacity a ukazuje procento. Jeho řádek
nemění výšku plátna mezi režimy. Paměťový kompromis tvoří nejvýše tři plátna
vzorkovaných dat a tři plátna obrazů rendereru; při limitu čtyř megapixelů
je to 48 MiB na každé straně mimo dekodéry a přechodné/GPU alokace.
Postup měření, skutečné testy a omezení:
[ověření komparátoru](../../verification/2026-11-08-comparator-navigation-and-images.md).

### Markdown tabulky a výběr písma (8. listopadu 2026)

Markdown viewer nabízí seznam nainstalovaných rodin písem, velikost a dosavadní
profily GitHub / Compact / Comfortable v jednom kompaktním řádku. Výběr se ukládá
odděleně od písma textového a zdrojového vieweru. Seznam připravuje a cachuje
backend z databáze rendereru; UI pouze zobrazuje model a odesílá změny.

Samostatný Markdown plugin podporuje GitHub pipe tabulky, zaškrtávací seznamy
a přeškrtnutí. Tabulky předává veřejným API jako omezené řádky a buňky se
zarovnáním, záhlavím a přesnými odkazy do zdrojových dat. Host na workeru vytváří
skutečné sloupce s rámečky a zalamováním buněk. Na úzkém okně zůstává minimální
čitelná šířka sloupce a je dostupný vodorovný posuvník. Změna písma nebo profilu
přepočítá již načtenou stránku bez opětovného čtení celého souboru.

Kód má pevnou šířku znaků nezávisle na písmu odstavců. Hledání, výběr a kopírování
pracují s viditelným textem a zachovávají zdrojové souřadnice. Nová metadata jsou
volitelná; starší host zobrazí prostý text řádků. Limity parseru, 64 sloupců,
stránkování a bezpečný textový fallback zůstávají explicitní. Obrázky používají
alternativní text, HTML je neaktivní; matematika, diagramy a vzdálené zdroje se
nevykonávají ani automaticky nestahují. Podrobnosti:
[prezentace viewerů](../../developer/VIEWER_PRESENTATION.md).

### Zpětná vazba panelů a reset nastavení (9. listopadu 2026)

Chyby využívají dostupnou šířku panelu, kopírování je vlevo dole a zavření vpravo
nahoře. Network ihned ukáže požadovanou lokaci s indikací načítání; indikace platí
i pro pomalý nativní disk a obnovu existujícího výpisu. Barvy souborů mají viditelný
seznam priorit. Reset všech stránek Options zachová karty, historii, oblíbené cesty
a vlastní příkazy; potvrzuje se přes OK a Cancel ho vrátí včetně motivu.
Fyzické klávesy Ctrl+[ / Ctrl+] a jejich Shift varianty respektují nastavení
zkratek i lokalizovanou klávesnici. Podrobnosti a ověření popisuje
[navazující návrh](2026-11-09-panel-options-polish.md).

### Velikost náhledů a rozdělená cache (9. listopadu 2026)

Options → Previews nabízí velikosti náhledů 48 až 256 logických pixelů;
výchozí hodnota je 72. Změna má náhled v obou panelech a respektuje OK, Cancel
a Reset. Větší velikost nezneplatní celou cache: jen nedostatečné snímky se
postupně nahradí, přičemž dosavadní náhled zůstane viditelný. Větší uložený
snímek lze použít pro menší velikost.

Disková cache používá adresáře podle prvních dvou hexadecimálních znaků hashe,
například `v1/ab/<hash>.mtc`. Vznikají až při zápisu; stará plochá cache zůstává
čitelná. Společné limity, úklid a bezpečnostní kontroly platí přes všechny
adresáře i oba formáty. Podrobnosti:
[velikost náhledů a cache](2026-11-09-thumbnail-size-and-cache.md).

### Výběr všeho a samostatný výběr souborů (9. listopadu 2026)

Ctrl+Shift+A označí zobrazené soubory i složky hlavní úrovně aktivního panelu,
včetně directory links/junctions. Samostatná akce `selection.files` / Select files
only má výchozí zkratku Ctrl+A na běžné klávesnici a nahradí výběr pouze soubory
hlavní úrovně; dříve označené složky a vnořené řádky odznačí.
Ctrl+D (`selection.clear-files`) odznačí soubory a zachová označené složky;
Ctrl+Shift+D (`selection.clear`) zruší celý výběr. Položka `..`
a pomocné neoznačitelné řádky stromu nejsou nikdy součástí výběru. Platí to také
pro filtrované seznamy, archivy a rozbalené větve. Ctrl+I nadále invertuje pouze
soubory a označení složek zachová.

Všechny čtyři akce jsou ve F10 → Selection a lze je přemapovat v Options → Keyboard.
Existující uživatelské mapování má přednost před novou výchozí zkratkou.
Pole pro zadávání textu si ponechávají vlastní chování Ctrl+A. UI poskytne
okamžitou zpětnou vazbu; backend zpracuje neměnné snapshoty a postupně
načítané seznamy bez blokování UI. Rozlišení akcí navazuje na Salamander
`src/mainwnd3.cpp` (`CM_ACTIVESELECTALL`, `CM_ACTIVEINVERTSEL`) a `src/menu4.cpp`,
ale nová zkratka nevyžaduje numerickou klávesnici.

### HTML viewer s Chromium (9. listopadu 2026)

HTML, HTM a XHTML otevírá F3 v samostatném volitelném CEF pluginu, jehož obraz
se zobrazuje uvnitř běžného viewer okna. Přípony a prioritu lze změnit v Options →
Plugins. Alt+F3 dál vynutí jednoduchý text. Plugin má vlastní Cargo projekt,
závislosti, testy a veřejný protokol; nesdílí privátní Rust typy aplikace.

Backend spravuje Chromium procesy, čtení a zachycené relativní prostředky z disku
i archivu; UI zobrazuje snímky a předává vstup. Výchozí režim je offline bez
JavaScriptu. Online režim zruší oprávnění k lokálním přílohám a vytvoří nový
kontext; JavaScript vyžaduje jeho výslovné zapnutí. Escape zavře viewer, Ctrl+F
hledá, zoom mění rozložení stránky. Mezerník v editačním poli zůstává textem.

CEF je v odděleném runtime balíčku, nezvětšuje hlavní EXE o Chromium knihovny.
Windows implementace prošla testem skutečného viewer okna. Linux má samostatný
launcher s cgroup omezením, ale nativní Chromium vykreslení ještě není ověřené.
Podrobnosti: [návrh](2026-11-09-html-viewer-design.md),
[veřejné API a bezpečnost](../../developer/WEB_VIEWER_API.md).

### Viewery při rozbalených větvích panelu (10. listopadu 2026)

F3 a Alt+F3 zůstávají dostupné pro běžné soubory hlavního výpisu i po rozbalení
složek. UI předá neměnnou projekci a identitu řádku; backend sestaví pořadí
viditelných souborů z ověřených vazeb na původní výpis. Mezerník tak respektuje
filtr a řazení zachycené při otevření vieweru. Pozdější navigace panelu jeho zdroj
nezmění. Nativní soubory dál čte nativní provider, členy archivu původní mount
s identitou člena a rodiče, včetně relativních prostředků HTML. Rozbalené větve
obsahují pouze složky; jejich zobrazovací cesty neopravňují k otevření souboru.
Připojení budoucích kontejnerů a dalších operací vyžaduje vlastní typované cíle.
Chování přechodu mezi soubory vychází ze Salamanderu `src/viewer3.cpp`,
`CM_NEXTFILE` / `GetNextFileNameForViewer`; zachycené pořadí je zde neměnné.

### Kopírování z rozbalených větví (10. listopadu 2026)

F5 zachytí označené řádky stromu včetně vnořených nativních a archivních složek.
Bez označení použije běžný řádek pod kurzorem; `..` a kontextové předky filtru
vyloučí. Backend před dialogem zachytí neměnný výběr a registrace providerů.
Pozdější navigace nezmění zdroje otevřeného návrhu. Potvrzení vstoupí do běžné
fronty a použije existující staging, rozhodování o konfliktech a úplné ověření
zápisu SHA-256. Zachycený seznam není snapshot obsahu disku.

Úspěšně dokončené kořenové položky se průběžně odznačují podle původní identity
výskytu a session. Změny výběru a projekce se zpracují v pořadí; opožděné dokončení
nesmí ovlivnit jinou složku. Clipboard a destruktivní operace vyžadují vlastní
navazující implementaci; F6 rozšiřuje následující kontrakt. Podrobný [návrh](2026-11-10-tree-copy-design.md),
[kontrakt](../../developer/TREE_PANEL.md#copy-from-expanded-panels) a
[ověření](../../verification/2026-11-10-tree-copy.md) uvádějí limity a aktuální stav.

### Přesun ze stromu a obnova po přenosu (10. listopadu 2026)

F6 zachytí nativní výběr se stejnou identitou jako F5, ale s neměnným záměrem
Move. Archivní a smíšené providery odmítne před přípravou. Použije běžný přesun:
každý zdrojový soubor odstraní až po nezávislém ověření cíle, prázdné složky
odstraňuje od potomků k rodičům. Přeskočené či chybové položky zůstávají označené;
pozdější chyba nevrací dříve dokončené soubory.

Dokončení Copy/Move, včetně chyby nebo zrušení, vyvolá obnovu kořene a již načtených
nativních větví obou stromových panelů. Backend zachová identitu session,
rozbalení, výběr, filtr a řazení; nepřenačítá celý panel do plochého režimu.
Zahrne i načtené sbalené větve, ale neotevírá dosud nenačtené složky ani cizí
archivní mounty. Čtení a použití výsledku chrání stejná brána jako zápisové
operace. Chyba čtení ponechá poslední známý výpis s diagnostikou.

Nové soubory kořene získají samostatný backendový snapshot pro F3 a Enter;
původní snapshot zůstává neměnnou autoritou pro přijetí session. Opožděné výsledky
nesmí změnit jinou navigaci. Stejnou údržbu nyní používá nativní ruční refresh
a změnové události kořene i rozbalených větví. Podrobnosti:
[přesun](2026-11-10-tree-move-design.md),
[obnova](2026-11-10-tree-transfer-reconciliation.md),
[sledování změn](2026-11-10-tree-watch-design.md).

Každý panel má jednu nativní skupinu nejvýše 32 nesestupných sledování, včetně
kořene. Ostatní způsobilé větve kontroluje backend postupně a spravedlivě pomocí
metadat. Události slučuje po sekundě klidu, nejdéle tři sekundy; pomalé čtení či
plná výstupní fronta mohou přidat další čekání. Periodická kontrola a opakování
chyby mají interval 15 sekund. Cíle pocházejí z přesně potvrzené projekce a drží
provider, nikdy se neodvozují z textu zobrazené cesty. Nová navigace okamžitě
zneplatní stará oznámení; fyzické uvolnění běží na backendu po návratu čtení.

### Zarovnání odkazů a chyb výpočtu velikosti (10. listopadu 2026)

Detailní a stručný panel rezervují stejný prostor pro rozbalovací ovládání i u
souborů a junctionů bez tlačítka. Jejich ikony a názvy se zarovnají s běžnými
složkami. Systémový `Documents and Settings` se nadále zobrazuje jako odkaz;
automatický výpočet nesleduje jeho cíl `C:\Users` ani ho nezapočítává podruhé.

Ikona chyby čtení stojí těsně vedle částečné velikosti. Roztažení sloupce nesmí
oddělit ikonu od čísla. Probíhající či zrušený výpočet sám o sobě není chybou;
neúplná čísla zůstávají odlišená od dokončených hodnot. Podrobnosti a aktuální
rozsah ověření uvádí [záznam kontroly rozložení](../../verification/2026-11-10-panel-row-alignment.md).

### Nastavení a potvrzovací ovládání (10. listopadu 2026)

Stránka General (dříve Panel) zahrnuje sekce Panels a Command line: zapamatování posledních 100
unikátních příkazů a vymazání historie. Samostatná stránka Command se odstraní
z bočního seznamu i z přepínače na úzkém okně. Historie zůstává součástí
rozpracovaného nastavení: OK uloží změny, Cancel obnoví původní hodnoty.

Všechny editory argumentů používají stejný výběrový seznam s malými tlačítky
`+`, `−`, `↑`, `↓` u jeho nadpisu. Hodnota vybraného argumentu se upravuje
samostatně. Pořadí, prázdné argumenty, mezery a Unicode zůstanou přesně zachované;
nejde o parsování textu příkazového shellu. Stejný vzor platí pro externí editor
i vlastní příkazy. Podrobnosti stanoví
[plán sjednocení ovládání](../plans/2026-11-10-settings-controls.md).

Copy, Move a Delete mají jako výchozí tlačítko provedení potvrzené operace.
U Copy/Move zůstává prvotní fokus v editaci cíle, Enter operaci potvrdí;
u mazání je fokus přímo na potvrzení. Escape, křížek a explicitně zvolené
Cancel nadále ruší dialog. Prázdný cíl nelze potvrdit a samotné zobrazení okna
nikdy neodesílá operaci. Trvalé smazání má stále odlišné výslovné potvrzení.

Ikony souborů **i složek** s atributem Hidden nebo System mají 50% neprůhlednost.
Text názvu, stavové informace a označení zůstávají plně čitelné. Rozhodují
metadata příslušného filesystemu, nikoli odhad podle názvu nebo drahé čtení
obsahu. Neznámé atributy nemají za následek zeslabení ikony.

Hlavičky detailního seznamu používají stejnou šířku jako řádky, včetně rezervy
pro posuvník. Hodnoty Size končí na společné pravé hraně. Datum změny (Modified)
a čas (Time) mají samostatné sloupce s pevnými společnými hranami; UI použije
oddělená lokalizovaná pole z backendu, nikdy nerozděluje formátovaný text podle
mezer. Oba časové sloupce řadí podle původního úplného času změny.

Aktualizované zkratky výběru (11. listopadu): Ctrl+A označí pouze soubory hlavní
úrovně, Ctrl+Shift+A také složky. Ctrl+D odznačí soubory včetně výslovně označených
vnořených souborů a zachová složky, Ctrl+Shift+D zruší celý výběr. Ctrl+I invertuje
jen soubory, Ctrl+Shift+I soubory i složky hlavní úrovně. Řádek `..` nikdy není
cílem výběru. Všechny akce lze přemapovat a F10 zobrazuje jejich aktuální účinné
zkratky včetně změn za běhu. Výslovná uživatelská přemapování mají nad novými
výchozími zkratkami přednost.

### Podrobnosti výstrah v panelech

Každá červená nebo žlutá výstraha v panelu má tooltip s konkrétním důvodem.
Kliknutí otevře úplnou diagnostiku; nemění výběr a neotevírá ani znovu nenačítá
označenou složku. Panel se aktivuje stejně jako při kliknutí na jeho ostatní
ovládací prvky. Platí to v detailním, stručném i náhledovém zobrazení, pro chyby
rozbalení stromu i pro chybové oznámení pod seznamem. Tooltip zůstává posunutý
mimo kurzor; běžné názvy souborů tooltip nadále nemají.

Žlutá výstraha u částečné velikosti znamená skutečně přeskočené nebo nečitelné
položky. Pouhé zrušení výpočtu ji nevytvoří. Výpočet pokračuje přes čitelné části
a report obsahuje přesné cesty, operace a důvody. Reporty jsou soukromé dočasné
soubory vytvářené backendem, nikoli neomezené seznamy chyb v paměti. Otevřený
report je neměnný snímek; další výpočet ani změna adresáře jeho obsah nezmění.
Čtení, zapisování i konečné uvolnění reportů probíhá mimo UI vlákno. Když nelze
uložit všechny podrobnosti, report výslovně ukáže počet chybějících záznamů.

Úplné reporty se otevírají v základním textovém vieweru s hledáním a kopírováním.
Krátké panelové chyby lze otevřít také ve stávajícím dialogu s tlačítkem pro
kopírování. Název reportu je pouze popisek a nikdy se nepoužije jako cesta k
otevření souboru. Stisk a uvolnění myši musí patřit stejnému řádku i reportu;
převázání virtualizovaného řádku nebo nový výpočet gesto zruší.

Zkrácený text panelové chyby a její úplný report mají společnou identitu výskytu.
Nová chyba se stejným krátkým textem nesmí otevřít starý report ani změnit cíl
rozpracovaného kliknutí. Kopírování úplných podrobností používá backendový zdroj,
ne zkrácený popisek panelu. Kontrakt popisuje
[Panel diagnostics](../../developer/PANEL_DIAGNOSTICS.md).

### Jednotky velikosti a menu řazení

Přehledné velikosti používají binární násobky 1024 s přesnými názvy KiB, MiB,
GiB a TiB. Například 1 MiB je 1 048 576 bajtů; MB by znamenalo 1 000 000 bajtů.
Počet bajtů a desetinný oddělovač se nadále formátují podle locale. Označení je
stejné pro panelové velikosti, volné místo, informace o disku a Copy/Move.
V menu řazení odděluje čára volbu Reverse order od výběru sloupce. Směr zůstává
samostatnou vlastností stejného řazení a oddělovač nemění indexy jeho položek.

### Textové viewery a soubory o velikosti 10 GB

Základní textový/binární viewer, zdrojový viewer včetně JSON a textový dokumentový
viewer smějí pro zobrazení načítat pouze omezený úsek kolem viewportu s malým
přednačtením. Velikost souboru nesmí určovat velikost alokace. První zobrazení ani
skok posuvníkem nesmí čekat na přečtení, parsování či indexování celého souboru.
Stejně se omezuje skrytá práce kvůli číslům řádků: vzdálený skok nesmí spustit
pročtení gigabajtů předchozího obsahu pouze kvůli údaji Ln/Col. Pokud nelze přesné
souřadnice zjistit levně z dostupného úseku nebo checkpointu, zobrazí se bajtová
pozice a nedostupné číslo řádku se nevymýšlí. Návrat do známé oblasti souřadnice
znovu zpřístupní.

Výslovně spuštěné hledání může postupně projít celý soubor, ale používá omezené
bloky a podporuje zrušení. Změna souboru během čtení dál podléhá kontrole identity
a revize. Člena komprimovaného archivu může být nutné nejprve rozbalit na soukromý
diskový spool pro náhodný přístup; nesmí se kvůli tomu celý držet v RAM a čas
dekomprese se nesmí vydávat za latenci běžného lokálního seeku.

Výkonnostní cíl je první použitelný snímek v řádu milisekund i pro 10GB lokální
soubor. Měření odděluje studené otevření/plugin/font inicializaci od opakovaného
posunu; uvádí přečtené bajty a paměť nezávislou na délce souboru. Testy mají
obsahovat skutečný vzdálený seek, dlouhé řádky, Unicode a virtuální i řídký 10GB
soubor. Číselný časový slib závisí na naměřeném úložišti a prostředí.

### Graf rychlosti Copy, Move a Delete

Detail vybrané úlohy v Jobs i automaticky otevřený průběh obsahují společný graf
rychlosti inspirovaný Průzkumníkem Windows. Copy/Move zobrazují skutečně zapsané
bajty/s a samostatnou křivku ověřovacích čtení, včetně dodatečných kontrol Move.
Jednotky jsou KiB/s, MiB/s atd. Delete zobrazuje zpracované položky/s včetně
ohlášených adresářů; systémový koš může hlásit pouze kořenové vybrané položky.
Kvůli grafu se nespouští další rekurzivní počítání velikostí.

Backend měří monotónním časem nejvýše čtyřikrát za sekundu a uchovává nejvýše
240 intervalů za celou operaci od začátku. Při zaplnění sousední intervaly sloučí
a zvětší také interval pro budoucí měření; uchová původní časové hranice a maxima
každé křivky. Časová osa se průběžně stlačuje, svislé měřítko zahrnuje i starší
špičky a zvětší se při novém maximu. Maxima různých křivek ve sloučeném intervalu
nemusela nastat současně. Aktuální rychlost v popisku zůstává skutečně naměřenou
rychlostí posledního intervalu. Čekání, zrušení a dokončení zobrazují nulovou
aktuální rychlost; již běžící systémové volání může po žádosti o zrušení dokončit
skutečnou práci, která se do historie stále započítá. Dokončený graf zůstává
součástí existujícího záznamu Jobs v aktuální relaci. Krátká operace má viditelný
bod i bez celého měřicího intervalu. Graf není procentem dokončení ani měřením
fyzické propustnosti disku. UI pouze formátuje omezené vzorky a přizpůsobuje graf
tématu a velikosti okna; nemění bezpečnostní ani kontrolní podmínky operací.
Podrobnosti: [návrh grafu](2026-11-10-operation-speed-chart-design.md).
